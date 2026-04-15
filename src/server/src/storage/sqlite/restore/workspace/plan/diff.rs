/**
@module PROJECTOR.SERVER.SQLITE_WORKSPACE_RESTORE_DIFF
Owns SQLite workspace-restore diff traversal, comparing current and reconstructed snapshots to produce per-document restore changes.
*/
// @fileimplements PROJECTOR.SERVER.SQLITE_WORKSPACE_RESTORE_DIFF
use std::collections::HashMap;

use projector_domain::{BootstrapSnapshot, DocumentId, ProvenanceEventKind};

use super::metadata::restore_change_metadata;

#[derive(Clone)]
pub(crate) struct WorkspaceRestoreChange {
    pub(crate) document_id: DocumentId,
    pub(crate) kind: ProvenanceEventKind,
    pub(crate) summary: String,
    pub(crate) path: WorkspaceRestorePathChange,
    pub(crate) body: Option<WorkspaceRestoreBodyChange>,
}

#[derive(Clone)]
pub(crate) struct WorkspaceRestorePathChange {
    pub(crate) mount_path: String,
    pub(crate) relative_path: String,
    pub(crate) deleted: bool,
    pub(crate) event_kind: String,
}

#[derive(Clone)]
pub(crate) struct WorkspaceRestoreBodyChange {
    pub(crate) base_text: String,
    pub(crate) body_text: String,
}

pub(crate) fn diff_workspace_restore_changes(
    current: &BootstrapSnapshot,
    restored: &BootstrapSnapshot,
    target_cursor: u64,
) -> Vec<WorkspaceRestoreChange> {
    let current_entries = current
        .manifest
        .entries
        .iter()
        .map(|entry| (entry.document_id.as_str().to_owned(), entry))
        .collect::<HashMap<_, _>>();
    let restored_entries = restored
        .manifest
        .entries
        .iter()
        .map(|entry| (entry.document_id.as_str().to_owned(), entry))
        .collect::<HashMap<_, _>>();
    let current_bodies = current
        .bodies
        .iter()
        .map(|body| (body.document_id.as_str().to_owned(), body.text.as_str()))
        .collect::<HashMap<_, _>>();
    let restored_bodies = restored
        .bodies
        .iter()
        .map(|body| (body.document_id.as_str().to_owned(), body.text.as_str()))
        .collect::<HashMap<_, _>>();

    let mut document_ids = current_entries
        .keys()
        .chain(restored_entries.keys())
        .cloned()
        .collect::<Vec<_>>();
    document_ids.sort();
    document_ids.dedup();

    let mut changes = Vec::new();
    for document_id in document_ids {
        let Some(restored_entry) = restored_entries.get(&document_id) else {
            continue;
        };
        let current_entry = current_entries.get(&document_id).copied();
        let current_body = current_bodies
            .get(&document_id)
            .copied()
            .unwrap_or_default();
        let restored_body = restored_bodies
            .get(&document_id)
            .copied()
            .unwrap_or_default();

        let current_live = current_entry.map(|entry| !entry.deleted).unwrap_or(false);
        let restored_live = !restored_entry.deleted;
        let path_changed = current_entry
            .map(|entry| {
                entry.mount_relative_path != restored_entry.mount_relative_path
                    || entry.relative_path != restored_entry.relative_path
            })
            .unwrap_or(restored_live);
        let body_changed = current_body != restored_body;

        let Some((kind, summary, path_event_kind)) = restore_change_metadata(
            restored_entry,
            current_live,
            restored_live,
            path_changed,
            body_changed,
            target_cursor,
        ) else {
            continue;
        };

        let body = if restored_live && body_changed {
            Some(WorkspaceRestoreBodyChange {
                base_text: current_body.to_owned(),
                body_text: restored_body.to_owned(),
            })
        } else {
            None
        };

        changes.push(WorkspaceRestoreChange {
            document_id: restored_entry.document_id.clone(),
            kind,
            summary,
            path: WorkspaceRestorePathChange {
                mount_path: restored_entry.mount_relative_path.display().to_string(),
                relative_path: restored_entry.relative_path.display().to_string(),
                deleted: restored_entry.deleted,
                event_kind: path_event_kind,
            },
            body,
        });
    }
    changes
}
