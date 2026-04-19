/**
@module PROJECTOR.SERVER.WORKSPACE_STORE
Owns the async workspace-store contract shared by the file-backed, SQLite, and Postgres server backends.
*/
// @fileimplements PROJECTOR.SERVER.WORKSPACE_STORE
use std::path::PathBuf;

use async_trait::async_trait;
use projector_domain::{
    BootstrapSnapshot, CreateDocumentRequest, DeleteDocumentRequest, DocumentBodyRevision,
    DocumentId, DocumentPathRevision, MoveDocumentRequest, ProvenanceEvent,
    PurgeDocumentBodyHistoryRequest, ResolveHistoricalPathRequest, RestoreDocumentBodyRevisionRequest, RestoreWorkspaceRequest,
    SyncEntryKind, SyncEntrySummary, UpdateDocumentRequest,
};

use super::StoreError;

#[async_trait]
pub trait WorkspaceStore: Send + Sync {
    async fn bootstrap_workspace(
        &self,
        workspace_id: &str,
        mounts: &[PathBuf],
        source_repo_name: Option<&str>,
        sync_entry_kind: Option<SyncEntryKind>,
    ) -> Result<(BootstrapSnapshot, u64), StoreError>;
    async fn list_sync_entries(&self, limit: usize) -> Result<Vec<SyncEntrySummary>, StoreError>;
    async fn changes_since(
        &self,
        workspace_id: &str,
        since_cursor: u64,
    ) -> Result<(BootstrapSnapshot, u64), StoreError>;
    async fn create_document(
        &self,
        request: &CreateDocumentRequest,
    ) -> Result<DocumentId, StoreError>;
    async fn update_document(&self, request: &UpdateDocumentRequest) -> Result<(), StoreError>;
    async fn delete_document(&self, request: &DeleteDocumentRequest) -> Result<(), StoreError>;
    async fn move_document(&self, request: &MoveDocumentRequest) -> Result<(), StoreError>;
    async fn list_events(
        &self,
        workspace_id: &str,
        limit: usize,
    ) -> Result<Vec<ProvenanceEvent>, StoreError>;
    async fn list_body_revisions(
        &self,
        workspace_id: &str,
        document_id: &str,
        limit: usize,
    ) -> Result<Vec<DocumentBodyRevision>, StoreError>;
    async fn list_path_revisions(
        &self,
        workspace_id: &str,
        document_id: &str,
        limit: usize,
    ) -> Result<Vec<DocumentPathRevision>, StoreError>;
    async fn reconstruct_workspace_at_cursor(
        &self,
        workspace_id: &str,
        cursor: u64,
    ) -> Result<BootstrapSnapshot, StoreError>;
    async fn restore_workspace_at_cursor(
        &self,
        request: &RestoreWorkspaceRequest,
    ) -> Result<(), StoreError>;
    async fn resolve_document_by_historical_path(
        &self,
        request: &ResolveHistoricalPathRequest,
    ) -> Result<DocumentId, StoreError>;
    async fn restore_document_body_revision(
        &self,
        request: &RestoreDocumentBodyRevisionRequest,
    ) -> Result<(), StoreError>;
    async fn purge_document_body_history(
        &self,
        request: &PurgeDocumentBodyHistoryRequest,
    ) -> Result<(), StoreError>;
}
