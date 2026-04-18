/**
@module PROJECTOR.SERVER.SQLITE_DOCUMENT_RESTORE_APPLY
Owns SQLite document-restore mutation and append-only history writes after a restore target and revision have been resolved.
*/
// @fileimplements PROJECTOR.SERVER.SQLITE_DOCUMENT_RESTORE_APPLY
use projector_domain::{ProvenanceEventKind, RestoreDocumentBodyRevisionRequest};

use crate::storage::body_persistence::{SnapshotBodyPersistence, SqliteBodyPersistence};
use crate::storage::body_state::{BodyStateModel, FULL_TEXT_BODY_MODEL};
use crate::storage::sqlite::state::{
    append_event, append_path_revision, make_event, save_workspace_state,
};
use crate::storage::{StoreError, history::FilePathRevision};

use super::resolve::DocumentRestoreResolution;

pub(super) fn apply_document_restore(
    transaction: &rusqlite::Transaction<'_>,
    request: &RestoreDocumentBodyRevisionRequest,
    mut resolution: DocumentRestoreResolution,
) -> Result<(), StoreError> {
    let entry = resolution.state.snapshot.manifest.entries[resolution.entry_index].clone();
    let current_state = FULL_TEXT_BODY_MODEL.state_from_materialized_text(
        resolution
            .state
            .snapshot
            .bodies
            .iter()
            .find(|body| body.document_id == resolution.document_id)
            .map(|body| body.text.clone())
            .unwrap_or_default(),
    );
    let body_persistence = SqliteBodyPersistence::new(transaction, &request.workspace_id);
    let target_state = resolution.target_state.clone();

    resolution.state.snapshot.manifest.entries[resolution.entry_index].deleted = false;
    resolution.state.snapshot.manifest.entries[resolution.entry_index].mount_relative_path =
        resolution.target_mount.clone();
    resolution.state.snapshot.manifest.entries[resolution.entry_index].relative_path =
        resolution.target_path.clone();
    body_persistence.write_current_state(
        &mut resolution.state.snapshot,
        &resolution.document_id,
        &target_state,
    );

    let event = make_event(
        &mut resolution.state,
        &request.actor_id,
        Some(resolution.document_id.clone()),
        Some(resolution.target_mount.display().to_string()),
        Some(resolution.target_path.display().to_string()),
        format!(
            "restored text document at {}/{} from body revision {}",
            resolution.target_mount.display(),
            resolution.target_path.display(),
            request.seq
        ),
        ProvenanceEventKind::DocumentUpdated,
    );
    save_workspace_state(transaction, &resolution.state)?;
    append_event(transaction, &request.workspace_id, &event)?;
    body_persistence.append_retained_history(
        event.cursor,
        &request.actor_id,
        &request.document_id,
        &FULL_TEXT_BODY_MODEL.restored_history(&current_state, &target_state),
        event.timestamp_ms,
    )?;
    if entry.deleted
        || resolution.target_mount != entry.mount_relative_path
        || resolution.target_path != entry.relative_path
    {
        append_path_revision(
            transaction,
            &request.workspace_id,
            &FilePathRevision {
                seq: event.cursor,
                workspace_cursor: event.cursor,
                actor_id: request.actor_id.clone(),
                document_id: request.document_id.clone(),
                mount_path: resolution.target_mount.display().to_string(),
                relative_path: resolution.target_path.display().to_string(),
                deleted: false,
                event_kind: "document_restored".to_owned(),
                timestamp_ms: event.timestamp_ms,
            },
        )?;
    }
    Ok(())
}
