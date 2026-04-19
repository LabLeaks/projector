/**
@module PROJECTOR.SERVER.POSTGRES_STORE
Owns the Postgres-backed workspace store adapter, connection/migration bootstrapping, and delegation into narrower Postgres server modules.
*/
// @fileimplements PROJECTOR.SERVER.POSTGRES_STORE
use async_trait::async_trait;
use projector_domain::{
    BootstrapSnapshot, CreateDocumentRequest, DeleteDocumentRequest, DocumentBodyPurgeMatch,
    DocumentBodyRedactionMatch, DocumentBodyRevision, DocumentId, DocumentPathRevision,
    MoveDocumentRequest, PreviewPurgeDocumentBodyHistoryRequest,
    PreviewRedactDocumentBodyHistoryRequest, ProvenanceEvent, PurgeDocumentBodyHistoryRequest,
    RedactDocumentBodyHistoryRequest, ResolveHistoricalPathRequest,
    RestoreDocumentBodyRevisionRequest, RestoreWorkspaceRequest, SyncEntryKind, SyncEntrySummary,
    UpdateDocumentRequest,
};
use std::path::PathBuf;
use tokio::sync::Mutex;
use tokio_postgres::{Client, NoTls};

use super::{StoreError, WorkspaceStore, bodies, history, manifest, provenance, workspaces};

const MIGRATIONS: &[&str] = &[
    include_str!("../../migrations/0001_init.sql"),
    include_str!("../../migrations/0002_history.sql"),
    include_str!("../../migrations/0003_history_workspace_cursor.sql"),
    include_str!("../../migrations/0004_sync_entry_metadata.sql"),
    include_str!("../../migrations/0005_body_storage_kinds.sql"),
    include_str!("../../migrations/0006_body_history_checkpoint_anchors.sql"),
];

pub struct PostgresWorkspaceStore {
    client: Mutex<Client>,
}

impl PostgresWorkspaceStore {
    pub async fn connect(connection_string: &str) -> Result<Self, StoreError> {
        let (client, connection) = tokio_postgres::connect(connection_string, NoTls).await?;
        tokio::spawn(async move {
            if let Err(err) = connection.await {
                eprintln!("projector-server postgres connection error: {err}");
            }
        });

        for migration in MIGRATIONS {
            client.batch_execute(migration).await?;
        }
        Ok(Self {
            client: Mutex::new(client),
        })
    }
}

#[async_trait]
impl WorkspaceStore for PostgresWorkspaceStore {
    async fn bootstrap_workspace(
        &self,
        workspace_id: &str,
        mounts: &[PathBuf],
        source_repo_name: Option<&str>,
        sync_entry_kind: Option<SyncEntryKind>,
    ) -> Result<(BootstrapSnapshot, u64), StoreError> {
        let mut client = self.client.lock().await;
        let transaction = client.transaction().await?;
        let result = workspaces::postgres_bootstrap_workspace(
            &transaction,
            workspace_id,
            mounts,
            source_repo_name,
            sync_entry_kind,
        )
        .await;
        transaction.commit().await?;
        result
    }

    async fn list_sync_entries(&self, limit: usize) -> Result<Vec<SyncEntrySummary>, StoreError> {
        let client = self.client.lock().await;
        workspaces::postgres_list_sync_entries(&client, limit).await
    }

    async fn changes_since(
        &self,
        workspace_id: &str,
        since_cursor: u64,
    ) -> Result<(BootstrapSnapshot, u64), StoreError> {
        let mut client = self.client.lock().await;
        let transaction = client.transaction().await?;
        let result =
            workspaces::postgres_changes_since(&transaction, workspace_id, since_cursor).await;
        transaction.commit().await?;
        result
    }

    async fn create_document(
        &self,
        request: &CreateDocumentRequest,
    ) -> Result<DocumentId, StoreError> {
        let mut client = self.client.lock().await;
        let transaction = client.transaction().await?;
        let result = manifest::postgres_create_document(&transaction, request).await;
        transaction.commit().await?;
        result
    }

    async fn update_document(&self, request: &UpdateDocumentRequest) -> Result<(), StoreError> {
        let mut client = self.client.lock().await;
        let transaction = client.transaction().await?;
        let result = bodies::postgres_update_document(&transaction, request).await;
        transaction.commit().await?;
        result
    }

    async fn delete_document(&self, request: &DeleteDocumentRequest) -> Result<(), StoreError> {
        let mut client = self.client.lock().await;
        let transaction = client.transaction().await?;
        let result = manifest::postgres_delete_document(&transaction, request).await;
        transaction.commit().await?;
        result
    }

    async fn move_document(&self, request: &MoveDocumentRequest) -> Result<(), StoreError> {
        let mut client = self.client.lock().await;
        let transaction = client.transaction().await?;
        let result = manifest::postgres_move_document(&transaction, request).await;
        transaction.commit().await?;
        result
    }

    async fn list_events(
        &self,
        workspace_id: &str,
        limit: usize,
    ) -> Result<Vec<ProvenanceEvent>, StoreError> {
        let client = self.client.lock().await;
        provenance::postgres_list_events(&client, workspace_id, limit).await
    }

    async fn list_body_revisions(
        &self,
        workspace_id: &str,
        document_id: &str,
        limit: usize,
    ) -> Result<Vec<DocumentBodyRevision>, StoreError> {
        let client = self.client.lock().await;
        history::postgres_list_body_revisions(&client, workspace_id, document_id, limit).await
    }

    async fn preview_redact_document_body_history(
        &self,
        request: &PreviewRedactDocumentBodyHistoryRequest,
    ) -> Result<Vec<DocumentBodyRedactionMatch>, StoreError> {
        let client = self.client.lock().await;
        history::postgres_preview_redact_document_body_history(&client, request).await
    }

    async fn preview_purge_document_body_history(
        &self,
        request: &PreviewPurgeDocumentBodyHistoryRequest,
    ) -> Result<Vec<DocumentBodyPurgeMatch>, StoreError> {
        let client = self.client.lock().await;
        history::postgres_preview_purge_document_body_history(&client, request).await
    }

    async fn list_path_revisions(
        &self,
        workspace_id: &str,
        document_id: &str,
        limit: usize,
    ) -> Result<Vec<DocumentPathRevision>, StoreError> {
        let client = self.client.lock().await;
        history::postgres_list_path_revisions(&client, workspace_id, document_id, limit).await
    }

    async fn reconstruct_workspace_at_cursor(
        &self,
        workspace_id: &str,
        cursor: u64,
    ) -> Result<BootstrapSnapshot, StoreError> {
        let client = self.client.lock().await;
        history::postgres_reconstruct_workspace_at_cursor(&client, workspace_id, cursor).await
    }

    async fn restore_workspace_at_cursor(
        &self,
        request: &RestoreWorkspaceRequest,
    ) -> Result<(), StoreError> {
        let mut client = self.client.lock().await;
        let transaction = client.transaction().await?;
        let result = history::postgres_restore_workspace_at_cursor(&transaction, request).await;
        if result.is_ok() {
            transaction.commit().await?;
        }
        result
    }

    async fn resolve_document_by_historical_path(
        &self,
        request: &ResolveHistoricalPathRequest,
    ) -> Result<DocumentId, StoreError> {
        let client = self.client.lock().await;
        history::postgres_resolve_document_by_historical_path(
            &client,
            &request.workspace_id,
            &request.mount_relative_path,
            &request.relative_path,
        )
        .await
    }

    async fn restore_document_body_revision(
        &self,
        request: &RestoreDocumentBodyRevisionRequest,
    ) -> Result<(), StoreError> {
        let mut client = self.client.lock().await;
        let transaction = client.transaction().await?;
        let result = bodies::postgres_restore_document_body_revision(&transaction, request).await;
        if result.is_ok() {
            transaction.commit().await?;
        }
        result
    }

    async fn redact_document_body_history(
        &self,
        request: &RedactDocumentBodyHistoryRequest,
    ) -> Result<(), StoreError> {
        let mut client = self.client.lock().await;
        let transaction = client.transaction().await?;
        let result = history::postgres_redact_document_body_history(&transaction, request).await;
        if result.is_ok() {
            transaction.commit().await?;
        }
        result
    }

    async fn purge_document_body_history(
        &self,
        request: &PurgeDocumentBodyHistoryRequest,
    ) -> Result<(), StoreError> {
        let mut client = self.client.lock().await;
        let transaction = client.transaction().await?;
        let result = history::postgres_purge_document_body_history(&transaction, request).await;
        if result.is_ok() {
            transaction.commit().await?;
        }
        result
    }
}
