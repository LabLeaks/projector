/**
@module PROJECTOR.SERVER.FILE_WORKSPACE_METADATA
Coordinates file-backed workspace metadata parsing, persistence, mount normalization, and sync-entry-kind decoding through narrower metadata helpers.
*/
// @fileimplements PROJECTOR.SERVER.FILE_WORKSPACE_METADATA
use std::path::{Path, PathBuf};

use projector_domain::SyncEntryKind;

use crate::storage::StoreError;

mod parse;
mod persist;

pub(super) use parse::{FileWorkspaceMetadata, parse_file_workspace_metadata};

pub(crate) fn file_read_bound_mounts(
    state_dir: &Path,
    workspace_id: &str,
) -> Result<Vec<PathBuf>, StoreError> {
    Ok(parse_file_workspace_metadata(state_dir, workspace_id)?.mounts)
}

pub(crate) fn normalize_mounts(mounts: &[PathBuf]) -> Vec<String> {
    let mut normalized = mounts
        .iter()
        .map(|mount| mount.display().to_string())
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    normalized
}

pub(crate) fn parse_sync_entry_kind(raw: &str) -> Result<SyncEntryKind, StoreError> {
    parse::parse_sync_entry_kind(raw)
}

pub(super) fn persist_file_workspace_metadata(
    state_dir: &Path,
    metadata: &FileWorkspaceMetadata,
) -> Result<(), StoreError> {
    persist::persist_file_workspace_metadata(state_dir, metadata)
}
