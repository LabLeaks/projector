/**
@module PROJECTOR.SERVER.WORKSPACES
Coordinates file-backed and Postgres-backed workspace bootstrap, sync-entry discovery, and delta reads through narrower backend-specific workspace modules.
*/
// @fileimplements PROJECTOR.SERVER.WORKSPACES
use std::path::{Path, PathBuf};

use projector_domain::{BootstrapSnapshot, SyncEntryKind};

use super::state_workspaces_root;

mod file;
mod postgres;

pub(crate) use file::{
    file_bootstrap_workspace, file_changes_since, file_list_sync_entries, file_read_bound_mounts,
    normalize_mounts, parse_sync_entry_kind,
};
pub(crate) use postgres::{
    postgres_bootstrap_workspace, postgres_changes_since, postgres_list_sync_entries,
};

pub(crate) fn workspace_dir(state_dir: &Path, workspace_id: &str) -> PathBuf {
    state_workspaces_root(state_dir).join(workspace_id)
}

pub(crate) fn infer_sync_entry_kind(snapshot: &BootstrapSnapshot) -> SyncEntryKind {
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

pub(crate) fn sync_entry_preview_summary(
    snapshot: &BootstrapSnapshot,
    kind: &SyncEntryKind,
) -> Option<String> {
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
