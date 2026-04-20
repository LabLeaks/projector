/**
@module PROJECTOR.SERVER.SQLITE_WORKSPACE_RESTORE_MERGE
Owns merge of the current live SQLite workspace snapshot with a reconstructed historical target snapshot into the restored live snapshot shape.
*/
// @fileimplements PROJECTOR.SERVER.SQLITE_WORKSPACE_RESTORE_MERGE
use std::collections::HashMap;

use projector_domain::{BootstrapSnapshot, ManifestState};

pub(super) fn build_restored_live_workspace_snapshot(
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
