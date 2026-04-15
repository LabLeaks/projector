/**
@module PROJECTOR.SERVER.FILE_WORKSPACES
Coordinates file-backed workspace bootstrap, sync-entry discovery, and delta reads through narrower file-workspace modules.
*/
// @fileimplements PROJECTOR.SERVER.FILE_WORKSPACES
mod bootstrap;
mod discovery;
mod metadata;

pub(crate) use bootstrap::{file_bootstrap_workspace, file_changes_since};
pub(crate) use discovery::file_list_sync_entries;
pub(crate) use metadata::{file_read_bound_mounts, normalize_mounts, parse_sync_entry_kind};
