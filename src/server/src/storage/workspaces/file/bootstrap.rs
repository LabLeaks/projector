/**
@module PROJECTOR.SERVER.FILE_WORKSPACE_BOOTSTRAP
Owns file-backed workspace bootstrap and changes-since reads over snapshot and provenance files.
*/
// @fileimplements PROJECTOR.SERVER.FILE_WORKSPACE_BOOTSTRAP
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use projector_domain::{BootstrapSnapshot, SyncEntryKind};

use crate::storage::bodies::{file_read_workspace_snapshot, snapshot_subset_for_documents};
use crate::storage::provenance::{file_read_workspace_events, file_workspace_cursor};
use crate::storage::{StoreError, state_workspaces_root};

use super::metadata::{
    FileWorkspaceMetadata, parse_file_workspace_metadata, persist_file_workspace_metadata,
};
use crate::storage::workspaces::workspace_dir;

pub(crate) fn file_bootstrap_workspace(
    state_dir: &Path,
    workspace_id: &str,
    mounts: &[PathBuf],
    source_repo_name: Option<&str>,
    sync_entry_kind: Option<SyncEntryKind>,
) -> Result<(BootstrapSnapshot, u64), StoreError> {
    fs::create_dir_all(state_workspaces_root(state_dir))?;
    let workspace_dir = workspace_dir(state_dir, workspace_id);
    fs::create_dir_all(&workspace_dir)?;
    let metadata_path = workspace_dir.join("metadata.txt");
    if metadata_path.exists() {
        let mut metadata = parse_file_workspace_metadata(state_dir, workspace_id)?;
        if metadata.mounts != mounts {
            return Err(StoreError::new(format!(
                "workspace {workspace_id} already bound to different mounts"
            )));
        }
        if metadata.source_repo_name.is_none() {
            metadata.source_repo_name = source_repo_name.map(str::to_owned);
        }
        if metadata.entry_kind.is_none() {
            metadata.entry_kind = sync_entry_kind;
        }
        persist_file_workspace_metadata(state_dir, &metadata)?;
    } else {
        persist_file_workspace_metadata(
            state_dir,
            &FileWorkspaceMetadata {
                workspace_id: workspace_id.to_owned(),
                mounts: mounts.to_vec(),
                source_repo_name: source_repo_name.map(str::to_owned),
                entry_kind: sync_entry_kind,
            },
        )?;
    }

    Ok((
        file_read_workspace_snapshot(state_dir, workspace_id)?,
        file_workspace_cursor(state_dir, workspace_id)?,
    ))
}

pub(crate) fn file_changes_since(
    state_dir: &Path,
    workspace_id: &str,
    since_cursor: u64,
) -> Result<(BootstrapSnapshot, u64), StoreError> {
    let current_cursor = file_workspace_cursor(state_dir, workspace_id)?;
    if current_cursor <= since_cursor {
        return Ok((BootstrapSnapshot::default(), current_cursor));
    }

    let snapshot = file_read_workspace_snapshot(state_dir, workspace_id)?;
    let events = file_read_workspace_events(state_dir, workspace_id)?;
    let changed_document_ids = events
        .into_iter()
        .filter(|event| event.cursor > since_cursor)
        .filter_map(|event| event.document_id)
        .collect::<HashSet<_>>();

    Ok((
        snapshot_subset_for_documents(&snapshot, &changed_document_ids),
        current_cursor,
    ))
}
