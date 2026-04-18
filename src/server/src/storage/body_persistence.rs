/**
@module PROJECTOR.SERVER.BODY_PERSISTENCE
Owns backend-specific adapters for loading current canonical body state, writing current body state, and appending retained body history so higher-level create, update, and restore flows do not manage body persistence directly.
*/
// @fileimplements PROJECTOR.SERVER.BODY_PERSISTENCE
use std::path::Path;

use async_trait::async_trait;
use projector_domain::{BootstrapSnapshot, DocumentId};

use super::StoreError;
use super::body_state::{
    BodyStateModel, CanonicalBodyState, CanonicalBodyStateKind, FULL_TEXT_BODY_MODEL,
    RetainedBodyHistoryPayload, body_state_from_snapshot, upsert_body_state,
};
use super::history::{FileBodyRevision, file_append_body_revision, insert_body_revision_tx};

pub(crate) trait SnapshotBodyPersistence {
    fn load_current_state(
        &self,
        snapshot: &BootstrapSnapshot,
        document_id: &DocumentId,
    ) -> CanonicalBodyState {
        body_state_from_snapshot(snapshot, document_id)
            .unwrap_or_else(|| FULL_TEXT_BODY_MODEL.empty_state())
    }

    fn write_current_state(
        &self,
        snapshot: &mut BootstrapSnapshot,
        document_id: &DocumentId,
        state: &CanonicalBodyState,
    ) {
        upsert_body_state(snapshot, document_id, state);
    }

    fn append_retained_history(
        &self,
        event_cursor: u64,
        actor_id: &str,
        document_id: &str,
        payload: &RetainedBodyHistoryPayload,
        timestamp_ms: u128,
    ) -> Result<(), StoreError>;
}

pub(crate) struct FileBodyPersistence<'a> {
    state_dir: &'a Path,
    workspace_id: &'a str,
}

impl<'a> FileBodyPersistence<'a> {
    pub(crate) fn new(state_dir: &'a Path, workspace_id: &'a str) -> Self {
        Self {
            state_dir,
            workspace_id,
        }
    }
}

impl SnapshotBodyPersistence for FileBodyPersistence<'_> {
    fn append_retained_history(
        &self,
        event_cursor: u64,
        actor_id: &str,
        document_id: &str,
        payload: &RetainedBodyHistoryPayload,
        timestamp_ms: u128,
    ) -> Result<(), StoreError> {
        file_append_body_revision(
            self.state_dir,
            self.workspace_id,
            FileBodyRevision::from_retained_history(
                event_cursor,
                event_cursor,
                actor_id.to_owned(),
                document_id.to_owned(),
                payload,
                timestamp_ms,
            ),
        )
    }
}

pub(crate) struct SqliteBodyPersistence<'a> {
    transaction: &'a rusqlite::Transaction<'a>,
    workspace_id: &'a str,
}

impl<'a> SqliteBodyPersistence<'a> {
    pub(crate) fn new(transaction: &'a rusqlite::Transaction<'a>, workspace_id: &'a str) -> Self {
        Self {
            transaction,
            workspace_id,
        }
    }
}

impl SnapshotBodyPersistence for SqliteBodyPersistence<'_> {
    fn append_retained_history(
        &self,
        event_cursor: u64,
        actor_id: &str,
        document_id: &str,
        payload: &RetainedBodyHistoryPayload,
        timestamp_ms: u128,
    ) -> Result<(), StoreError> {
        crate::storage::sqlite::state::append_body_revision(
            self.transaction,
            self.workspace_id,
            &FileBodyRevision::from_retained_history(
                event_cursor,
                event_cursor,
                actor_id.to_owned(),
                document_id.to_owned(),
                payload,
                timestamp_ms,
            ),
        )
    }
}

#[async_trait]
pub(crate) trait AsyncBodyPersistence {
    async fn load_current_state(&self, document_id: &str)
    -> Result<CanonicalBodyState, StoreError>;

    async fn write_current_state(
        &self,
        document_id: &str,
        state: &CanonicalBodyState,
    ) -> Result<(), StoreError>;

    async fn append_retained_history(
        &self,
        event_cursor: u64,
        actor_id: &str,
        document_id: &str,
        payload: &RetainedBodyHistoryPayload,
    ) -> Result<(), StoreError>;
}

pub(crate) struct PostgresBodyPersistence<'a> {
    transaction: &'a tokio_postgres::Transaction<'a>,
    workspace_id: &'a str,
}

impl<'a> PostgresBodyPersistence<'a> {
    pub(crate) fn new(
        transaction: &'a tokio_postgres::Transaction<'a>,
        workspace_id: &'a str,
    ) -> Self {
        Self {
            transaction,
            workspace_id,
        }
    }
}

#[async_trait]
impl AsyncBodyPersistence for PostgresBodyPersistence<'_> {
    async fn load_current_state(
        &self,
        document_id: &str,
    ) -> Result<CanonicalBodyState, StoreError> {
        Ok(self
            .transaction
            .query_opt(
                "select state_kind, body_text from document_body_snapshots where workspace_id = $1 and document_id = $2",
                &[&self.workspace_id, &document_id],
            )
            .await?
            .map(|row| {
                let kind = CanonicalBodyStateKind::parse(row.get::<_, String>("state_kind").as_str())
                    .map_err(StoreError::new)?;
                Ok::<CanonicalBodyState, StoreError>(FULL_TEXT_BODY_MODEL.state_from_storage_record(
                    kind,
                    row.get::<_, String>("body_text"),
                ))
            })
            .transpose()?
            .unwrap_or_else(|| FULL_TEXT_BODY_MODEL.empty_state()))
    }

    async fn write_current_state(
        &self,
        document_id: &str,
        state: &CanonicalBodyState,
    ) -> Result<(), StoreError> {
        self.transaction
            .execute(
                "insert into document_body_snapshots \
                 (document_id, workspace_id, state_kind, body_text, compacted_through_seq) \
                 values ($1, $2, $3, $4, 0) \
                 on conflict (document_id) do update set state_kind = excluded.state_kind, body_text = excluded.body_text, updated_at = now()",
                &[
                    &document_id,
                    &self.workspace_id,
                    &state.kind().as_str(),
                    &state.materialized_text(),
                ],
            )
            .await?;
        Ok(())
    }

    async fn append_retained_history(
        &self,
        event_cursor: u64,
        actor_id: &str,
        document_id: &str,
        payload: &RetainedBodyHistoryPayload,
    ) -> Result<(), StoreError> {
        insert_body_revision_tx(
            self.transaction,
            self.workspace_id,
            document_id,
            event_cursor,
            actor_id,
            payload,
        )
        .await
    }
}
