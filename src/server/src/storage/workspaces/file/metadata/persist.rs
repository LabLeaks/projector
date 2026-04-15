/**
@module PROJECTOR.SERVER.FILE_WORKSPACE_METADATA_PERSIST
Owns persistence and encoding of file-backed workspace metadata records.
*/
// @fileimplements PROJECTOR.SERVER.FILE_WORKSPACE_METADATA_PERSIST
use std::fs;
use std::path::Path;

use projector_domain::SyncEntryKind;

use crate::storage::StoreError;
use crate::storage::workspaces::workspace_dir;

use super::parse::FileWorkspaceMetadata;

pub(super) fn persist_file_workspace_metadata(
    state_dir: &Path,
    metadata: &FileWorkspaceMetadata,
) -> Result<(), StoreError> {
    let metadata_path = workspace_dir(state_dir, &metadata.workspace_id).join("metadata.txt");
    let mut content = format!("workspace_id={}\n", metadata.workspace_id);
    for mount in &metadata.mounts {
        content.push_str(&format!("projection_relative_path={}\n", mount.display()));
    }
    if let Some(source_repo_name) = &metadata.source_repo_name {
        content.push_str(&format!("source_repo_name={source_repo_name}\n"));
    }
    if let Some(entry_kind) = &metadata.entry_kind {
        content.push_str(&format!(
            "entry_kind={}\n",
            format_sync_entry_kind(entry_kind)
        ));
    }
    fs::write(metadata_path, content)?;
    Ok(())
}

fn format_sync_entry_kind(kind: &SyncEntryKind) -> &'static str {
    match kind {
        SyncEntryKind::File => "file",
        SyncEntryKind::Directory => "directory",
    }
}
