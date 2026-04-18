/**
@module PROJECTOR.SERVER.SQLITE_DOCUMENT_RESTORE_APPLY
Owns SQLite document-restore mutation and append-only history writes after a restore target and revision have been resolved.
*/
// @fileimplements PROJECTOR.SERVER.SQLITE_DOCUMENT_RESTORE_APPLY
use projector_domain::{ProvenanceEventKind, RestoreDocumentBodyRevisionRequest};

use crate::storage::body_state::{CanonicalBodyState, RetainedBodyHistoryPayload};
use crate::storage::sqlite::state::{
    append_body_revision, append_event, append_path_revision, make_event, save_workspace_state,
    upsert_body_state,
};
use crate::storage::{
    StoreError,
    history::{FileBodyRevision, FilePathRevision},
};

use super::resolve::DocumentRestoreResolution;

pub(super) fn apply_document_restore(
    transaction: &rusqlite::Transaction<'_>,
    request: &RestoreDocumentBodyRevisionRequest,
    mut resolution: DocumentRestoreResolution,
) -> Result<(), StoreError> {
    let entry = resolution.state.snapshot.manifest.entries[resolution.entry_index].clone();
    let current_text = resolution
        .state
        .snapshot
        .bodies
        .iter()
        .find(|body| body.document_id == resolution.document_id)
        .map(|body| body.text.clone())
        .unwrap_or_default();

    resolution.state.snapshot.manifest.entries[resolution.entry_index].deleted = false;
    resolution.state.snapshot.manifest.entries[resolution.entry_index].mount_relative_path =
        resolution.target_mount.clone();
    resolution.state.snapshot.manifest.entries[resolution.entry_index].relative_path =
        resolution.target_path.clone();
    upsert_body_state(
        &mut resolution.state.snapshot,
        &resolution.document_id,
        &CanonicalBodyState::full_text_merge_v1(resolution.target_revision.body_text.clone()),
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
    append_body_revision(
        transaction,
        &request.workspace_id,
        &FileBodyRevision::from_retained_history(
            event.cursor,
            event.cursor,
            request.actor_id.clone(),
            request.document_id.clone(),
            &RetainedBodyHistoryPayload::full_text_revision_v1(
                current_text,
                resolution.target_revision.body_text,
                false,
            ),
            event.timestamp_ms,
        ),
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
