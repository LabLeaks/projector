/**
@module PROJECTOR.SERVER.FILE_STORE
Owns the file-backed workspace store adapter and snapshot-write helper over the narrower file workspace, manifest, body, provenance, and history modules.
*/
// @fileimplements PROJECTOR.SERVER.FILE_STORE
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use projector_domain::{
    BootstrapSnapshot, CreateDocumentRequest, DeleteDocumentRequest, DocumentBodyRevision,
    DocumentId, DocumentPathRevision, MoveDocumentRequest, ProvenanceEvent,
    PurgeDocumentBodyHistoryRequest, RedactDocumentBodyHistoryRequest,
    ResolveHistoricalPathRequest,
    RestoreDocumentBodyRevisionRequest, RestoreWorkspaceRequest,
    SyncEntryKind, SyncEntrySummary, UpdateDocumentRequest,
};

use super::{StoreError, WorkspaceStore, bodies, history, manifest, provenance, workspaces};

#[derive(Clone, Debug)]
pub struct FileWorkspaceStore {
    state_dir: PathBuf,
}

impl FileWorkspaceStore {
    pub fn new(state_dir: impl Into<PathBuf>) -> Self {
        Self {
            state_dir: state_dir.into(),
        }
    }

    pub fn write_workspace_snapshot(
        &self,
        workspace_id: &str,
        snapshot: &BootstrapSnapshot,
    ) -> Result<(), StoreError> {
        let previous = bodies::file_read_workspace_snapshot(&self.state_dir, workspace_id)?;
        bodies::file_persist_workspace_snapshot(&self.state_dir, workspace_id, snapshot)?;
        let changed_events = provenance::synthetic_events_for_snapshot_change(
            &previous,
            snapshot,
            provenance::file_workspace_cursor(&self.state_dir, workspace_id)?,
        );
        if !changed_events.is_empty() {
            provenance::file_extend_workspace_events(
                &self.state_dir,
                workspace_id,
                changed_events,
            )?;
        }
        Ok(())
    }
}

pub fn write_workspace_snapshot(
    state_dir: &Path,
    workspace_id: &str,
    snapshot: &BootstrapSnapshot,
) -> Result<(), StoreError> {
    FileWorkspaceStore::new(state_dir).write_workspace_snapshot(workspace_id, snapshot)
}

#[async_trait]
impl WorkspaceStore for FileWorkspaceStore {
    async fn bootstrap_workspace(
        &self,
        workspace_id: &str,
        mounts: &[PathBuf],
        source_repo_name: Option<&str>,
        sync_entry_kind: Option<SyncEntryKind>,
    ) -> Result<(BootstrapSnapshot, u64), StoreError> {
        workspaces::file_bootstrap_workspace(
            &self.state_dir,
            workspace_id,
            mounts,
            source_repo_name,
            sync_entry_kind,
        )
    }

    async fn list_sync_entries(&self, limit: usize) -> Result<Vec<SyncEntrySummary>, StoreError> {
        workspaces::file_list_sync_entries(&self.state_dir, limit)
    }

    async fn changes_since(
        &self,
        workspace_id: &str,
        since_cursor: u64,
    ) -> Result<(BootstrapSnapshot, u64), StoreError> {
        workspaces::file_changes_since(&self.state_dir, workspace_id, since_cursor)
    }

    async fn create_document(
        &self,
        request: &CreateDocumentRequest,
    ) -> Result<DocumentId, StoreError> {
        manifest::file_create_document(&self.state_dir, request)
    }

    async fn update_document(&self, request: &UpdateDocumentRequest) -> Result<(), StoreError> {
        bodies::file_update_document(&self.state_dir, request)
    }

    async fn delete_document(&self, request: &DeleteDocumentRequest) -> Result<(), StoreError> {
        manifest::file_delete_document(&self.state_dir, request)
    }

    async fn move_document(&self, request: &MoveDocumentRequest) -> Result<(), StoreError> {
        manifest::file_move_document(&self.state_dir, request)
    }

    async fn list_events(
        &self,
        workspace_id: &str,
        limit: usize,
    ) -> Result<Vec<ProvenanceEvent>, StoreError> {
        provenance::file_list_events(&self.state_dir, workspace_id, limit)
    }

    async fn list_body_revisions(
        &self,
        workspace_id: &str,
        document_id: &str,
        limit: usize,
    ) -> Result<Vec<DocumentBodyRevision>, StoreError> {
        history::file_list_body_revisions(&self.state_dir, workspace_id, document_id, limit)
    }

    async fn list_path_revisions(
        &self,
        workspace_id: &str,
        document_id: &str,
        limit: usize,
    ) -> Result<Vec<DocumentPathRevision>, StoreError> {
        history::file_list_path_revisions(&self.state_dir, workspace_id, document_id, limit)
    }

    async fn reconstruct_workspace_at_cursor(
        &self,
        workspace_id: &str,
        cursor: u64,
    ) -> Result<BootstrapSnapshot, StoreError> {
        history::file_reconstruct_workspace_at_cursor(&self.state_dir, workspace_id, cursor)
    }

    async fn restore_workspace_at_cursor(
        &self,
        request: &RestoreWorkspaceRequest,
    ) -> Result<(), StoreError> {
        history::file_restore_workspace_at_cursor(&self.state_dir, request)
    }

    async fn resolve_document_by_historical_path(
        &self,
        request: &ResolveHistoricalPathRequest,
    ) -> Result<DocumentId, StoreError> {
        history::file_resolve_document_by_historical_path(
            &self.state_dir,
            &request.workspace_id,
            &request.mount_relative_path,
            &request.relative_path,
        )
    }

    async fn restore_document_body_revision(
        &self,
        request: &RestoreDocumentBodyRevisionRequest,
    ) -> Result<(), StoreError> {
        bodies::file_restore_document_body_revision(&self.state_dir, request)
    }

    async fn redact_document_body_history(
        &self,
        request: &RedactDocumentBodyHistoryRequest,
    ) -> Result<(), StoreError> {
        history::file_redact_document_body_history(&self.state_dir, request)
    }

    async fn purge_document_body_history(
        &self,
        request: &PurgeDocumentBodyHistoryRequest,
    ) -> Result<(), StoreError> {
        history::file_purge_document_body_history(&self.state_dir, request)
    }
}
