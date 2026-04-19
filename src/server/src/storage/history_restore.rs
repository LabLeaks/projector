/**
@module PROJECTOR.SERVER.HISTORY_RESTORE
Owns workspace-history reconstruction deltas and live-snapshot restore planning above backend-specific history reads and writes.
*/
// @fileimplements PROJECTOR.SERVER.HISTORY_RESTORE
use std::collections::HashMap;

use projector_domain::{
    BootstrapSnapshot, DocumentId, ManifestEntry, ManifestState, ProvenanceEventKind,
};

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

pub(crate) fn build_restored_live_workspace_snapshot(
    current: &BootstrapSnapshot,
    target: &BootstrapSnapshot,
) -> BootstrapSnapshot {
    let current_entries = current
        .manifest
        .entries
        .iter()
        .cloned()
        .map(|entry| (entry.document_id.as_str().to_owned(), entry))
        .collect::<HashMap<_, _>>();
    let target_entries = target
        .manifest
        .entries
        .iter()
        .cloned()
        .map(|entry| (entry.document_id.as_str().to_owned(), entry))
        .collect::<HashMap<_, _>>();

    let mut entries = current_entries.clone();
    for (document_id, target_entry) in &target_entries {
        if !target_entry.deleted {
            entries.insert(document_id.clone(), target_entry.clone());
            continue;
        }
        if let Some(current_entry) = entries.get_mut(document_id) {
            current_entry.deleted = true;
        } else {
            entries.insert(document_id.clone(), target_entry.clone());
        }
    }
    for (document_id, current_entry) in &current_entries {
        if !target_entries.contains_key(document_id) && !current_entry.deleted {
            let mut deleted_entry = current_entry.clone();
            deleted_entry.deleted = true;
            entries.insert(document_id.clone(), deleted_entry);
        }
    }

    let mut manifest_entries = entries.into_values().collect::<Vec<_>>();
    manifest_entries.sort_by(|left, right| {
        left.mount_relative_path
            .cmp(&right.mount_relative_path)
            .then_with(|| left.relative_path.cmp(&right.relative_path))
            .then_with(|| left.document_id.as_str().cmp(right.document_id.as_str()))
    });

    let mut bodies = target.bodies.clone();
    bodies.sort_by(|left, right| left.document_id.as_str().cmp(right.document_id.as_str()));

    BootstrapSnapshot {
        manifest: ManifestState {
            entries: manifest_entries,
        },
        bodies,
    }
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
            current_entry,
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

fn restore_change_metadata(
    current_entry: Option<&ManifestEntry>,
    restored_entry: &ManifestEntry,
    current_live: bool,
    restored_live: bool,
    path_changed: bool,
    body_changed: bool,
    target_cursor: u64,
) -> Option<(ProvenanceEventKind, String, String)> {
    let path_display = format!(
        "{}/{}",
        restored_entry.mount_relative_path.display(),
        restored_entry.relative_path.display()
    );
    if current_live && !restored_live {
        return Some((
            ProvenanceEventKind::DocumentDeleted,
            format!(
                "workspace restore to cursor {target_cursor} removed text document from live workspace at {path_display}"
            ),
            "document_deleted".to_owned(),
        ));
    }
    if !current_live && restored_live {
        return Some((
            ProvenanceEventKind::DocumentCreated,
            format!(
                "workspace restore to cursor {target_cursor} restored text document at {path_display}"
            ),
            "workspace_restored".to_owned(),
        ));
    }
    if current_live && restored_live && path_changed {
        return Some((
            ProvenanceEventKind::DocumentMoved,
            format!(
                "workspace restore to cursor {target_cursor} moved text document to {path_display}"
            ),
            "workspace_restored".to_owned(),
        ));
    }
    if current_live && restored_live && body_changed {
        return Some((
            ProvenanceEventKind::DocumentUpdated,
            format!(
                "workspace restore to cursor {target_cursor} restored text document body at {path_display}"
            ),
            "workspace_restored".to_owned(),
        ));
    }
    let _ = current_entry;
    None
}
