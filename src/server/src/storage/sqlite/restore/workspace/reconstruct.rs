/**
@module PROJECTOR.SERVER.SQLITE_WORKSPACE_RECONSTRUCTION
Owns SQLite reconstruction of a workspace snapshot at a historical cursor from append-only path and body history.
*/
// @fileimplements PROJECTOR.SERVER.SQLITE_WORKSPACE_RECONSTRUCTION
use std::collections::HashMap;

use projector_domain::{BootstrapSnapshot, DocumentId, DocumentKind, ManifestEntry, ManifestState};

use crate::storage::body_projection::snapshot_from_manifest_entries;
use crate::storage::history_compaction::replay_body_revision_run;
use crate::storage::sqlite::history::{read_body_revisions, read_path_history};
use crate::storage::sqlite::state::effective_workspace_cursor;
use crate::storage::{StoreError, history::FilePathRevision};

pub(super) fn reconstruct_workspace_at_cursor(
    connection: &rusqlite::Connection,
    workspace_id: &str,
    cursor: u64,
) -> Result<BootstrapSnapshot, StoreError> {
    let path_history = read_path_history(connection, workspace_id)?;
    let body_history = read_body_revisions(connection, workspace_id)?;

    let latest_paths = latest_path_revisions(path_history, cursor);
    let latest_bodies = replay_body_revision_run(body_history.into_iter().filter(|revision| {
        effective_workspace_cursor(revision.seq, revision.workspace_cursor) <= cursor
    }));

    let mut entries = latest_paths
        .into_values()
        .map(|revision| ManifestEntry {
            document_id: DocumentId::new(revision.document_id),
            mount_relative_path: revision.mount_path.into(),
            relative_path: revision.relative_path.into(),
            kind: DocumentKind::Text,
            deleted: revision.deleted,
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        left.mount_relative_path
            .cmp(&right.mount_relative_path)
            .then_with(|| left.relative_path.cmp(&right.relative_path))
            .then_with(|| left.document_id.as_str().cmp(right.document_id.as_str()))
    });

    Ok(snapshot_from_manifest_entries(entries, |document_id| {
        latest_bodies.get(document_id.as_str()).cloned()
    }))
}

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

fn latest_path_revisions(
    path_history: Vec<FilePathRevision>,
    cursor: u64,
) -> HashMap<String, FilePathRevision> {
    path_history
        .into_iter()
        .filter(|revision| {
            effective_workspace_cursor(revision.seq, revision.workspace_cursor) <= cursor
        })
        .fold(
            HashMap::<String, FilePathRevision>::new(),
            |mut acc, revision| {
                let replace = acc
                    .get(&revision.document_id)
                    .map(|current| {
                        effective_workspace_cursor(revision.seq, revision.workspace_cursor)
                            > effective_workspace_cursor(current.seq, current.workspace_cursor)
                            || (effective_workspace_cursor(revision.seq, revision.workspace_cursor)
                                == effective_workspace_cursor(
                                    current.seq,
                                    current.workspace_cursor,
                                )
                                && revision.seq > current.seq)
                    })
                    .unwrap_or(true);
                if replace {
                    acc.insert(revision.document_id.clone(), revision);
                }
                acc
            },
        )
}
