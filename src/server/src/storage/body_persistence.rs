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
    RetainedBodyHistoryPayload, YrsTextCheckpoint, body_state_from_snapshot, upsert_body_state,
};
use super::history::{
    FileBodyRevision, file_append_body_revision, file_read_body_revisions, insert_body_revision_tx,
    latest_checkpoint_anchor_seq,
};

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
        let checkpoint_anchor_seq =
            if payload.kind() == super::body_state::RetainedBodyHistoryKind::YrsTextUpdateV1 {
                latest_checkpoint_anchor_seq(
                    file_read_body_revisions(self.state_dir, self.workspace_id)?,
                    document_id,
                )
            } else {
                Some(event_cursor)
            };
        file_append_body_revision(
            self.state_dir,
            self.workspace_id,
            FileBodyRevision::from_retained_history(
                event_cursor,
                event_cursor,
                actor_id.to_owned(),
                document_id.to_owned(),
                checkpoint_anchor_seq,
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
        let checkpoint_anchor_seq = if payload.kind()
            == super::body_state::RetainedBodyHistoryKind::YrsTextUpdateV1
        {
            latest_checkpoint_anchor_seq(
                crate::storage::sqlite::read_body_revisions(self.transaction, self.workspace_id)?,
                document_id,
            )
        } else {
            Some(event_cursor)
        };
        crate::storage::sqlite::state::append_body_revision(
            self.transaction,
            self.workspace_id,
            &FileBodyRevision::from_retained_history(
                event_cursor,
                event_cursor,
                actor_id.to_owned(),
                document_id.to_owned(),
                checkpoint_anchor_seq,
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
        let Some(row) = self
            .transaction
            .query_opt(
                "select state_kind, body_text, yjs_state, compacted_through_seq \
                 from document_body_snapshots where workspace_id = $1 and document_id = $2",
                &[&self.workspace_id, &document_id],
            )
            .await?
        else {
            return Ok(FULL_TEXT_BODY_MODEL.empty_state());
        };

        let kind = CanonicalBodyStateKind::parse(row.get::<_, String>("state_kind").as_str())
            .map_err(StoreError::new)?;
        let body_text = row.get::<_, String>("body_text");
        let compacted_through_seq = row.get::<_, i64>("compacted_through_seq") as u64;

        if kind != CanonicalBodyStateKind::YrsTextCheckpointV1 {
            return FULL_TEXT_BODY_MODEL
                .state_from_storage_record(kind, body_text)
                .map_err(StoreError::new);
        }

        let base_checkpoint = if let Some(yjs_state) = row.get::<_, Option<Vec<u8>>>("yjs_state") {
            YrsTextCheckpoint::from_checkpoint_bytes(yjs_state).map_err(StoreError::new)?
        } else {
            YrsTextCheckpoint::from_storage_payload(&body_text).map_err(StoreError::new)?
        };
        let update_rows = self
            .transaction
            .query(
                "select update_blob \
                 from document_body_updates \
                 where workspace_id = $1 and document_id = $2 and seq > $3 \
                 order by seq asc",
                &[
                    &self.workspace_id,
                    &document_id,
                    &(compacted_through_seq as i64),
                ],
            )
            .await?;
        let checkpoint = update_rows
            .into_iter()
            .try_fold(base_checkpoint, |checkpoint, row| {
                checkpoint.with_update_v1(&row.get::<_, Vec<u8>>("update_blob"))
            })
            .map_err(StoreError::new)?;
        FULL_TEXT_BODY_MODEL
            .state_from_yrs_checkpoint(checkpoint)
            .map_err(StoreError::new)
    }

    async fn write_current_state(
        &self,
        document_id: &str,
        state: &CanonicalBodyState,
    ) -> Result<(), StoreError> {
        self.transaction
            .execute(
                "insert into document_body_snapshots \
                 (document_id, workspace_id, state_kind, body_text, yjs_state, state_vector, compacted_through_seq) \
                 values ($1, $2, $3, $4, null, null, 0) \
                 on conflict (document_id) do update set \
                   state_kind = excluded.state_kind, \
                   body_text = excluded.body_text, \
                   updated_at = now()",
                &[
                    &document_id,
                    &self.workspace_id,
                    &state.kind().as_str(),
                    &state.storage_payload(),
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
        let checkpoint_anchor_seq =
            if payload.kind() == super::body_state::RetainedBodyHistoryKind::YrsTextUpdateV1 {
                self.transaction
                    .query_opt(
                        "select seq, checkpoint_anchor_seq, history_kind \
                     from document_body_revisions \
                     where workspace_id = $1 and document_id = $2 \
                     order by seq desc \
                     limit 1",
                        &[&self.workspace_id, &document_id],
                    )
                    .await?
                    .and_then(|row| {
                        row.get::<_, Option<i64>>("checkpoint_anchor_seq")
                            .map(|seq| seq as u64)
                            .or_else(|| match row.get::<_, String>("history_kind").as_str() {
                                "yrs_text_update_v1" => None,
                                _ => Some(row.get::<_, i64>("seq") as u64),
                            })
                    })
            } else {
                Some(event_cursor)
            };
        insert_body_revision_tx(
            self.transaction,
            self.workspace_id,
            document_id,
            event_cursor,
            actor_id,
            checkpoint_anchor_seq,
            payload,
        )
        .await?;

        if let Some(update_blob) = payload.yrs_update_v1_bytes().map_err(StoreError::new)? {
            self.transaction
                .execute(
                    "insert into document_body_updates (document_id, workspace_id, actor_id, update_blob) \
                     values ($1, $2, $3, $4)",
                    &[&document_id, &self.workspace_id, &actor_id, &update_blob],
                )
                .await?;
        } else {
            sync_postgres_checkpoint_metadata(self.transaction, self.workspace_id, document_id)
                .await?;
        }
        Ok(())
    }
}

async fn sync_postgres_checkpoint_metadata(
    transaction: &tokio_postgres::Transaction<'_>,
    workspace_id: &str,
    document_id: &str,
) -> Result<(), StoreError> {
    let Some(snapshot_row) = transaction
        .query_opt(
            "select state_kind, body_text from document_body_snapshots where workspace_id = $1 and document_id = $2",
            &[&workspace_id, &document_id],
        )
        .await?
    else {
        return Ok(());
    };
    let kind = CanonicalBodyStateKind::parse(snapshot_row.get::<_, String>("state_kind").as_str())
        .map_err(StoreError::new)?;
    let body_text = snapshot_row.get::<_, String>("body_text");
    let state = FULL_TEXT_BODY_MODEL
        .state_from_storage_record(kind, body_text)
        .map_err(StoreError::new)?;
    let (yjs_state, state_vector) = postgres_checkpoint_artifacts(&state)?;
    let compacted_through_seq = transaction
        .query_one(
            "select coalesce(max(seq), 0) as max_seq from document_body_updates \
             where workspace_id = $1 and document_id = $2",
            &[&workspace_id, &document_id],
        )
        .await?
        .get::<_, i64>("max_seq");
    transaction
        .execute(
            "update document_body_snapshots \
             set yjs_state = $3, state_vector = $4, compacted_through_seq = $5, updated_at = now() \
             where workspace_id = $1 and document_id = $2",
            &[
                &workspace_id,
                &document_id,
                &yjs_state,
                &state_vector,
                &compacted_through_seq,
            ],
        )
        .await?;
    if compacted_through_seq > 0 {
        transaction
            .execute(
                "delete from document_body_updates \
                 where workspace_id = $1 and document_id = $2 and seq <= $3",
                &[&workspace_id, &document_id, &compacted_through_seq],
            )
            .await?;
    }
    Ok(())
}

fn postgres_checkpoint_artifacts(
    state: &CanonicalBodyState,
) -> Result<(Option<Vec<u8>>, Option<Vec<u8>>), StoreError> {
    match state.kind() {
        CanonicalBodyStateKind::FullTextMergeV1 => Ok((None, None)),
        CanonicalBodyStateKind::YrsTextCheckpointV1 => {
            let checkpoint = YrsTextCheckpoint::from_storage_payload(state.storage_payload())
                .map_err(StoreError::new)?;
            Ok((
                Some(checkpoint.checkpoint_bytes().to_vec()),
                Some(checkpoint.state_vector_v1().map_err(StoreError::new)?),
            ))
        }
    }
}
