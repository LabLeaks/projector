/**
@module PROJECTOR.SERVER.WORKSPACES
Coordinates file-backed and Postgres-backed workspace bootstrap, sync-entry discovery, and delta reads through narrower backend-specific workspace modules.
*/
// @fileimplements PROJECTOR.SERVER.WORKSPACES
use std::path::{Path, PathBuf};

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
