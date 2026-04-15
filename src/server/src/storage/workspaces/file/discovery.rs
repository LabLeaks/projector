/**
@module PROJECTOR.SERVER.FILE_SYNC_ENTRY_DISCOVERY
Owns file-backed remote sync-entry listing, kind inference, and preview rendering for `projector get`.
*/
// @fileimplements PROJECTOR.SERVER.FILE_SYNC_ENTRY_DISCOVERY
use std::fs;
use std::path::Path;

use projector_domain::{BootstrapSnapshot, SyncEntryKind, SyncEntrySummary};

use crate::storage::bodies::file_read_workspace_snapshot;
use crate::storage::provenance::file_read_workspace_events;
use crate::storage::{StoreError, state_workspaces_root};

use super::metadata::parse_file_workspace_metadata;

pub(crate) fn file_list_sync_entries(
    state_dir: &Path,
    limit: usize,
) -> Result<Vec<SyncEntrySummary>, StoreError> {
    let mut entries = Vec::new();
    let root = state_workspaces_root(state_dir);
    if !root.exists() {
        return Ok(entries);
    }

    for dir in fs::read_dir(root)? {
        let dir = dir?;
        if !dir.file_type()?.is_dir() {
            continue;
        }
        let workspace_id = dir.file_name().to_string_lossy().into_owned();
        let metadata = match parse_file_workspace_metadata(state_dir, &workspace_id) {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        let Some(remote_path) = metadata.mounts.first() else {
            continue;
        };
        let snapshot = file_read_workspace_snapshot(state_dir, &workspace_id)?;
        let events = file_read_workspace_events(state_dir, &workspace_id).unwrap_or_default();
        let kind = metadata
            .entry_kind
            .unwrap_or_else(|| infer_snapshot_kind(&snapshot));
        entries.push(SyncEntrySummary {
            sync_entry_id: workspace_id.clone(),
            workspace_id,
            remote_path: remote_path.display().to_string(),
            kind: kind.clone(),
            source_repo_name: metadata.source_repo_name,
            last_updated_ms: events.last().map(|event| event.timestamp_ms),
            preview: preview_summary(&snapshot, &kind),
        });
    }

    entries.sort_by(|left, right| {
        right
            .last_updated_ms
            .unwrap_or(0)
            .cmp(&left.last_updated_ms.unwrap_or(0))
            .then(left.remote_path.cmp(&right.remote_path))
    });
    if entries.len() > limit {
        entries.truncate(limit);
    }
    Ok(entries)
}

fn infer_snapshot_kind(snapshot: &BootstrapSnapshot) -> SyncEntryKind {
    if snapshot
        .manifest
        .entries
        .iter()
        .any(|entry| !entry.deleted && entry.relative_path.as_os_str().is_empty())
    {
        SyncEntryKind::File
    } else {
        SyncEntryKind::Directory
    }
}

fn preview_summary(snapshot: &BootstrapSnapshot, kind: &SyncEntryKind) -> Option<String> {
    match kind {
        SyncEntryKind::File => snapshot.bodies.first().map(|body| {
            let single_line = body.text.split_whitespace().collect::<Vec<_>>().join(" ");
            single_line.chars().take(120).collect::<String>()
        }),
        SyncEntryKind::Directory => {
            let live_count = snapshot
                .manifest
                .entries
                .iter()
                .filter(|entry| !entry.deleted)
                .count();
            if live_count == 0 {
                None
            } else if let Some(first_entry) = snapshot
                .manifest
                .entries
                .iter()
                .filter(|entry| !entry.deleted)
                .min_by(|left, right| left.relative_path.cmp(&right.relative_path))
            {
                Some(format!(
                    "{live_count} files; first={}",
                    first_entry.relative_path.display()
                ))
            } else {
                Some(format!("{live_count} files"))
            }
        }
    }
}
