/**
@module PROJECTOR.SERVER.SQLITE_WORKSPACE_RESTORE_APPLY
Owns SQLite application of planned workspace rewind changes onto live workspace state and append-only history rows.
*/
// @fileimplements PROJECTOR.SERVER.SQLITE_WORKSPACE_RESTORE_APPLY
use projector_domain::RestoreWorkspaceRequest;

use crate::storage::body_state::RetainedBodyHistoryPayload;
use crate::storage::sqlite::state::{
    append_body_revision, append_event, append_path_revision, load_required_workspace_state,
    make_event, save_workspace_state,
};
use crate::storage::{StoreError, history::FileBodyRevision, history::FilePathRevision};

use super::plan::diff::diff_workspace_restore_changes;
use super::reconstruct::{build_restored_live_workspace_snapshot, reconstruct_workspace_at_cursor};

pub(super) fn restore_workspace_at_cursor_tx(
    transaction: &rusqlite::Transaction<'_>,
    request: &RestoreWorkspaceRequest,
) -> Result<(), StoreError> {
    let current_state = load_required_workspace_state(transaction, &request.workspace_id)?;
    let provided_cursor = request
        .based_on_cursor
        .ok_or_else(|| StoreError::new("workspace restore missing based_on_cursor precondition"))?;
    if provided_cursor != current_state.cursor {
        return Err(StoreError::conflict(
            "stale_cursor",
            format!(
                "workspace restore based on stale cursor {provided_cursor}; current workspace cursor is {}",
                current_state.cursor
            ),
        ));
    }
    if request.cursor > current_state.cursor {
        return Err(StoreError::new(format!(
            "workspace restore target cursor {} is newer than current workspace cursor {}",
            request.cursor, current_state.cursor
        )));
    }

    let target_snapshot =
        reconstruct_workspace_at_cursor(transaction, &request.workspace_id, request.cursor)?;
    let restored_snapshot =
        build_restored_live_workspace_snapshot(&current_state.snapshot, &target_snapshot);
    let changes =
        diff_workspace_restore_changes(&current_state.snapshot, &restored_snapshot, request.cursor);

    let mut state = current_state;
    state.snapshot = restored_snapshot;
    for change in changes {
        let event = make_event(
            &mut state,
            &request.actor_id,
            Some(change.document_id.clone()),
            Some(change.path.mount_path.clone()),
            Some(change.path.relative_path.clone()),
            change.summary.clone(),
            change.kind.clone(),
        );
        append_event(transaction, &request.workspace_id, &event)?;
        if let Some(body) = change.body {
            append_body_revision(
                transaction,
                &request.workspace_id,
                &FileBodyRevision::from_retained_history(
                    event.cursor,
                    event.cursor,
                    request.actor_id.clone(),
                    change.document_id.as_str().to_owned(),
                    &RetainedBodyHistoryPayload::full_text_revision_v1(
                        body.base_text,
                        body.body_text,
                        false,
                    ),
                    event.timestamp_ms,
                ),
            )?;
        }
        append_path_revision(
            transaction,
            &request.workspace_id,
            &FilePathRevision {
                seq: event.cursor,
                workspace_cursor: event.cursor,
                actor_id: request.actor_id.clone(),
                document_id: change.document_id.as_str().to_owned(),
                mount_path: change.path.mount_path,
                relative_path: change.path.relative_path,
                deleted: change.path.deleted,
                event_kind: change.path.event_kind,
                timestamp_ms: event.timestamp_ms,
            },
        )?;
    }
    save_workspace_state(transaction, &state)
}
