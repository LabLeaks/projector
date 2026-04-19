/**
@module PROJECTOR.SERVER.SQLITE_DOCUMENT_RESTORE_RESOLUTION
Owns SQLite document-restore target resolution, including live-entry lookup, requested revision lookup, and path-conflict checks.
*/
// @fileimplements PROJECTOR.SERVER.SQLITE_DOCUMENT_RESTORE_RESOLUTION
use std::path::PathBuf;

use projector_domain::{DocumentId, RestoreDocumentBodyRevisionRequest};

use crate::storage::sqlite::history::read_body_revisions;
use crate::storage::sqlite::state::load_required_workspace_state;
use crate::storage::{
    StoreError, body_state::CanonicalBodyState, history::replay_body_revision_run,
};

pub(super) struct DocumentRestoreResolution {
    pub(super) state: crate::storage::sqlite::state::SqliteWorkspaceState,
    pub(super) document_id: DocumentId,
    pub(super) entry_index: usize,
    pub(super) target_mount: PathBuf,
    pub(super) target_path: PathBuf,
    pub(super) target_state: CanonicalBodyState,
}

pub(super) fn resolve_document_restore_target(
    transaction: &rusqlite::Transaction<'_>,
    request: &RestoreDocumentBodyRevisionRequest,
) -> Result<DocumentRestoreResolution, StoreError> {
    let state = load_required_workspace_state(transaction, &request.workspace_id)?;
    let document_id = DocumentId::new(&request.document_id);
    let Some(entry_index) = state
        .snapshot
        .manifest
        .entries
        .iter()
        .position(|entry| entry.document_id == document_id)
    else {
        return Err(StoreError::new(format!(
            "document {} is not present in workspace {}",
            request.document_id, request.workspace_id
        )));
    };
    let entry = state.snapshot.manifest.entries[entry_index].clone();
    let target_mount = request
        .target_mount_relative_path
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| entry.mount_relative_path.clone());
    let target_path = request
        .target_relative_path
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| entry.relative_path.clone());

    if (entry.deleted
        || target_mount != entry.mount_relative_path
        || target_path != entry.relative_path)
        && state.snapshot.manifest.entries.iter().any(|candidate| {
            candidate.document_id != document_id
                && !candidate.deleted
                && candidate.mount_relative_path == target_mount
                && candidate.relative_path == target_path
        })
    {
        return Err(StoreError::conflict(
            "path_taken",
            format!(
                "document already exists at {}/{}",
                target_mount.display(),
                target_path.display()
            ),
        ));
    }

    let body_revisions = read_body_revisions(transaction, &request.workspace_id)?;
    if !body_revisions
        .iter()
        .any(|revision| revision.document_id == request.document_id && revision.seq == request.seq)
    {
        return Err(StoreError::new(format!(
            "document {} has no body revision {}",
            request.document_id, request.seq
        )));
    }
    let target_revisions = body_revisions
        .into_iter()
        .filter(|revision| {
            revision.document_id == request.document_id && revision.seq <= request.seq
        })
        .collect::<Vec<_>>();
    let fallback_target_state = target_revisions
        .last()
        .ok_or_else(|| {
            StoreError::new(format!(
                "document {} has no body revision {}",
                request.document_id, request.seq
            ))
        })?
        .materialized_body_state();
    let replayed_target_state = replay_body_revision_run(target_revisions.into_iter())
        .remove(request.document_id.as_str())
        .ok_or_else(|| {
            StoreError::new(format!(
                "document {} has no body revision {}",
                request.document_id, request.seq
            ))
        })?;
    let target_state =
        if replayed_target_state.materialized_text() == fallback_target_state.materialized_text() {
            replayed_target_state
        } else {
            fallback_target_state
        };

    Ok(DocumentRestoreResolution {
        state,
        document_id,
        entry_index,
        target_mount,
        target_path,
        target_state,
    })
}
