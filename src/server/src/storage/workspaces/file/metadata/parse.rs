/**
@module PROJECTOR.SERVER.FILE_WORKSPACE_METADATA_PARSE
Owns file-backed workspace metadata parsing and sync-entry-kind decoding.
*/
// @fileimplements PROJECTOR.SERVER.FILE_WORKSPACE_METADATA_PARSE
use std::fs;
use std::path::{Path, PathBuf};

use projector_domain::SyncEntryKind;

use crate::storage::StoreError;
use crate::storage::workspaces::workspace_dir;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct FileWorkspaceMetadata {
    pub(crate) workspace_id: String,
    pub(crate) mounts: Vec<PathBuf>,
    pub(crate) source_repo_name: Option<String>,
    pub(crate) entry_kind: Option<SyncEntryKind>,
}

pub(crate) fn parse_sync_entry_kind(raw: &str) -> Result<SyncEntryKind, StoreError> {
    match raw {
        "file" => Ok(SyncEntryKind::File),
        "directory" => Ok(SyncEntryKind::Directory),
        other => Err(StoreError::new(format!("unknown sync entry kind {other}"))),
    }
}

pub(crate) fn parse_file_workspace_metadata(
    state_dir: &Path,
    workspace_id: &str,
) -> Result<FileWorkspaceMetadata, StoreError> {
    let metadata_path = workspace_dir(state_dir, workspace_id).join("metadata.txt");
    if !metadata_path.exists() {
        return Err(StoreError::new(format!(
            "workspace {workspace_id} is not bound"
        )));
    }

    let mut metadata = FileWorkspaceMetadata {
        workspace_id: workspace_id.to_owned(),
        ..FileWorkspaceMetadata::default()
    };
    for line in fs::read_to_string(metadata_path)?.lines() {
        if let Some(value) = line.strip_prefix("projection_relative_path=") {
            metadata.mounts.push(PathBuf::from(value));
            continue;
        }
        if let Some(value) = line.strip_prefix("source_repo_name=") {
            metadata.source_repo_name = Some(value.to_owned());
            continue;
        }
        if let Some(value) = line.strip_prefix("entry_kind=") {
            metadata.entry_kind = Some(parse_sync_entry_kind(value)?);
        }
    }
    Ok(metadata)
}
