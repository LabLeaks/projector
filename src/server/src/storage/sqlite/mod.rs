/**
@module PROJECTOR.SERVER.SQLITE_STORAGE
Implements the blessed single-user BYO server store over one SQLite database file by delegating workspace, manifest, history, and restore concerns to narrower SQLite storage modules.
*/
// @fileimplements PROJECTOR.SERVER.SQLITE_STORAGE
use std::path::Path;
use std::sync::Mutex;

use async_trait::async_trait;
use projector_domain::{
    BootstrapSnapshot, CreateDocumentRequest, DeleteDocumentRequest, DocumentBodyRevision,
    DocumentId, DocumentPathRevision, MoveDocumentRequest, ProvenanceEvent,
    PurgeDocumentBodyHistoryRequest, RedactDocumentBodyHistoryRequest,
    ResolveHistoricalPathRequest,
    RestoreDocumentBodyRevisionRequest, RestoreWorkspaceRequest,
    SyncEntryKind, SyncEntrySummary, UpdateDocumentRequest,
};
use rusqlite::Connection;

use super::{StoreError, WorkspaceStore};

mod history;
mod manifest;
mod restore;
pub(crate) mod state;
mod workspace;

pub(crate) use history::read_body_revisions;

pub struct SqliteWorkspaceStore {
    connection: Mutex<Connection>,
}

impl SqliteWorkspaceStore {
    pub fn connect(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let connection = Connection::open(path)?;
        connection.execute_batch(state::SQLITE_SCHEMA)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }
}

#[async_trait]
impl WorkspaceStore for SqliteWorkspaceStore {
    async fn bootstrap_workspace(
        &self,
        workspace_id: &str,
        mounts: &[std::path::PathBuf],
        source_repo_name: Option<&str>,
        sync_entry_kind: Option<SyncEntryKind>,
    ) -> Result<(BootstrapSnapshot, u64), StoreError> {
        let mut connection = self.connection.lock().expect("sqlite mutex poisoned");
        let transaction = connection.transaction()?;
        let state = workspace::bootstrap_workspace_tx(
            &transaction,
            workspace_id,
            mounts,
            source_repo_name,
            sync_entry_kind,
        )?;
        transaction.commit()?;
        Ok((state.snapshot, state.cursor))
    }

    async fn list_sync_entries(&self, limit: usize) -> Result<Vec<SyncEntrySummary>, StoreError> {
        let connection = self.connection.lock().expect("sqlite mutex poisoned");
        workspace::list_sync_entries(&connection, limit)
    }

    async fn changes_since(
        &self,
        workspace_id: &str,
        since_cursor: u64,
    ) -> Result<(BootstrapSnapshot, u64), StoreError> {
        let connection = self.connection.lock().expect("sqlite mutex poisoned");
        workspace::changes_since(&connection, workspace_id, since_cursor)
    }

    async fn create_document(
        &self,
        request: &CreateDocumentRequest,
    ) -> Result<DocumentId, StoreError> {
        let mut connection = self.connection.lock().expect("sqlite mutex poisoned");
        let transaction = connection.transaction()?;
        let document_id = manifest::create_document_tx(&transaction, request)?;
        transaction.commit()?;
        Ok(document_id)
    }

    async fn update_document(&self, request: &UpdateDocumentRequest) -> Result<(), StoreError> {
        let mut connection = self.connection.lock().expect("sqlite mutex poisoned");
        let transaction = connection.transaction()?;
        manifest::update_document_tx(&transaction, request)?;
        transaction.commit()?;
        Ok(())
    }

    async fn delete_document(&self, request: &DeleteDocumentRequest) -> Result<(), StoreError> {
        let mut connection = self.connection.lock().expect("sqlite mutex poisoned");
        let transaction = connection.transaction()?;
        manifest::delete_document_tx(&transaction, request)?;
        transaction.commit()?;
        Ok(())
    }

    async fn move_document(&self, request: &MoveDocumentRequest) -> Result<(), StoreError> {
        let mut connection = self.connection.lock().expect("sqlite mutex poisoned");
        let transaction = connection.transaction()?;
        manifest::move_document_tx(&transaction, request)?;
        transaction.commit()?;
        Ok(())
    }

    async fn list_events(
        &self,
        workspace_id: &str,
        limit: usize,
    ) -> Result<Vec<ProvenanceEvent>, StoreError> {
        let connection = self.connection.lock().expect("sqlite mutex poisoned");
        history::read_recent_events(&connection, workspace_id, limit)
    }

    async fn list_body_revisions(
        &self,
        workspace_id: &str,
        document_id: &str,
        limit: usize,
    ) -> Result<Vec<DocumentBodyRevision>, StoreError> {
        let connection = self.connection.lock().expect("sqlite mutex poisoned");
        history::list_body_revisions(&connection, workspace_id, document_id, limit)
    }

    async fn list_path_revisions(
        &self,
        workspace_id: &str,
        document_id: &str,
        limit: usize,
    ) -> Result<Vec<DocumentPathRevision>, StoreError> {
        let connection = self.connection.lock().expect("sqlite mutex poisoned");
        history::list_path_revisions(&connection, workspace_id, document_id, limit)
    }

    async fn reconstruct_workspace_at_cursor(
        &self,
        workspace_id: &str,
        cursor: u64,
    ) -> Result<BootstrapSnapshot, StoreError> {
        let connection = self.connection.lock().expect("sqlite mutex poisoned");
        restore::reconstruct_workspace_at_cursor(&connection, workspace_id, cursor)
    }

    async fn restore_workspace_at_cursor(
        &self,
        request: &RestoreWorkspaceRequest,
    ) -> Result<(), StoreError> {
        let mut connection = self.connection.lock().expect("sqlite mutex poisoned");
        let transaction = connection.transaction()?;
        restore::restore_workspace_at_cursor_tx(&transaction, request)?;
        transaction.commit()?;
        Ok(())
    }

    async fn resolve_document_by_historical_path(
        &self,
        request: &ResolveHistoricalPathRequest,
    ) -> Result<DocumentId, StoreError> {
        let connection = self.connection.lock().expect("sqlite mutex poisoned");
        history::resolve_document_by_historical_path(
            &connection,
            &request.workspace_id,
            &request.mount_relative_path,
            &request.relative_path,
        )
    }

    async fn restore_document_body_revision(
        &self,
        request: &RestoreDocumentBodyRevisionRequest,
    ) -> Result<(), StoreError> {
        let mut connection = self.connection.lock().expect("sqlite mutex poisoned");
        let transaction = connection.transaction()?;
        restore::restore_document_body_revision_tx(&transaction, request)?;
        transaction.commit()?;
        Ok(())
    }

    async fn redact_document_body_history(
        &self,
        request: &RedactDocumentBodyHistoryRequest,
    ) -> Result<(), StoreError> {
        let mut connection = self.connection.lock().expect("sqlite mutex poisoned");
        let transaction = connection.transaction()?;
        history::redact_document_body_history(&transaction, request)?;
        transaction.commit()?;
        Ok(())
    }

    async fn purge_document_body_history(
        &self,
        request: &PurgeDocumentBodyHistoryRequest,
    ) -> Result<(), StoreError> {
        let mut connection = self.connection.lock().expect("sqlite mutex poisoned");
        let transaction = connection.transaction()?;
        history::purge_document_body_history(&transaction, request)?;
        transaction.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use projector_domain::{ActorId, WorkspaceId};

    use super::*;

    fn temp_db_path(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time before unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("projector-{name}-{unique}.sqlite3"))
    }

    #[tokio::test]
    async fn sqlite_store_bootstraps_and_creates_documents() {
        let db_path = temp_db_path("bootstrap-create");
        let store = SqliteWorkspaceStore::connect(&db_path).expect("sqlite store");
        let workspace_id = WorkspaceId::new("ws-1");
        let actor_id = ActorId::new("actor-a");

        let (snapshot, cursor) = store
            .bootstrap_workspace(
                workspace_id.as_str(),
                &[PathBuf::from("private")],
                Some("demo-repo"),
                Some(SyncEntryKind::Directory),
            )
            .await
            .expect("bootstrap workspace");
        assert!(snapshot.manifest.entries.is_empty());
        assert_eq!(cursor, 0);

        let document_id = store
            .create_document(&CreateDocumentRequest {
                workspace_id: workspace_id.as_str().to_owned(),
                actor_id: actor_id.as_str().to_owned(),
                based_on_cursor: Some(cursor),
                mount_relative_path: "private".to_owned(),
                relative_path: "notes.md".to_owned(),
                text: "hello".to_owned(),
            })
            .await
            .expect("create document");

        let (snapshot, cursor) = store
            .bootstrap_workspace(
                workspace_id.as_str(),
                &[PathBuf::from("private")],
                Some("demo-repo"),
                Some(SyncEntryKind::Directory),
            )
            .await
            .expect("bootstrap workspace again");
        assert_eq!(cursor, 1);
        assert_eq!(snapshot.manifest.entries.len(), 1);
        assert_eq!(snapshot.bodies.len(), 1);
        assert_eq!(snapshot.bodies[0].document_id, document_id);
    }

    #[tokio::test]
    async fn sqlite_store_lists_sync_entries() {
        let db_path = temp_db_path("list-sync-entries");
        let store = SqliteWorkspaceStore::connect(&db_path).expect("sqlite store");

        let (_snapshot, cursor) = store
            .bootstrap_workspace(
                "ws-1",
                &[PathBuf::from("private")],
                Some("demo-repo"),
                Some(SyncEntryKind::Directory),
            )
            .await
            .expect("bootstrap workspace");

        store
            .create_document(&CreateDocumentRequest {
                workspace_id: "ws-1".to_owned(),
                actor_id: "actor-a".to_owned(),
                based_on_cursor: Some(cursor),
                mount_relative_path: "private".to_owned(),
                relative_path: "notes.md".to_owned(),
                text: "hello world".to_owned(),
            })
            .await
            .expect("create document");

        let entries = store
            .list_sync_entries(10)
            .await
            .expect("list sync entries");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].workspace_id, "ws-1");
        assert_eq!(entries[0].remote_path, "private");
        assert_eq!(entries[0].source_repo_name.as_deref(), Some("demo-repo"));
    }
}
