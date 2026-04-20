/**
@module PROJECTOR.SERVER.HISTORY
Owns shared retained-history record types and insert helpers while delegating file-backed and Postgres-backed history adapters to narrower backend modules.
*/
// @fileimplements PROJECTOR.SERVER.HISTORY
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use super::StoreError;
use super::body_state::{
    BodyStateModel, CanonicalBodyState, FULL_TEXT_BODY_MODEL, RetainedBodyHistoryKind,
    RetainedBodyHistoryPayload,
};
use super::history_surgery::build_redaction_preview_lines;
use projector_domain::{DocumentBodyRedactionMatch, DocumentBodyRevision, DocumentPathEventKind};

pub(crate) use super::history_file::*;
pub(crate) use super::history_postgres::*;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct FileBodyRevision {
    pub seq: u64,
    #[serde(default)]
    pub workspace_cursor: u64,
    pub actor_id: String,
    pub document_id: String,
    #[serde(default)]
    pub checkpoint_anchor_seq: Option<u64>,
    #[serde(default = "default_retained_body_history_kind")]
    pub history_kind: RetainedBodyHistoryKind,
    pub base_text: String,
    pub body_text: String,
    pub conflicted: bool,
    pub timestamp_ms: u128,
}

fn default_retained_body_history_kind() -> RetainedBodyHistoryKind {
    RetainedBodyHistoryKind::FullTextRevisionV1
}

impl FileBodyRevision {
    pub(crate) fn from_retained_history(
        seq: u64,
        workspace_cursor: u64,
        actor_id: impl Into<String>,
        document_id: impl Into<String>,
        checkpoint_anchor_seq: Option<u64>,
        payload: &RetainedBodyHistoryPayload,
        timestamp_ms: u128,
    ) -> Self {
        Self {
            seq,
            workspace_cursor,
            actor_id: actor_id.into(),
            document_id: document_id.into(),
            checkpoint_anchor_seq,
            history_kind: payload.kind(),
            base_text: payload.base_text().to_owned(),
            body_text: payload.storage_payload().to_owned(),
            conflicted: payload.conflicted(),
            timestamp_ms,
        }
    }

    pub(crate) fn retained_history(&self) -> Result<RetainedBodyHistoryPayload, StoreError> {
        FULL_TEXT_BODY_MODEL
            .history_from_storage_record(
                self.history_kind,
                self.base_text.clone(),
                self.body_text.clone(),
                self.conflicted,
            )
            .map_err(StoreError::new)
    }

    pub(crate) fn materialized_body_state(&self) -> Result<CanonicalBodyState, StoreError> {
        Ok(self.retained_history()?.materialized_body_state())
    }

    pub(crate) fn replayed_body_state(
        &self,
        previous_state: Option<&CanonicalBodyState>,
    ) -> Result<CanonicalBodyState, StoreError> {
        Ok(self.retained_history()?.replayed_body_state(previous_state))
    }

    pub(crate) fn effective_checkpoint_anchor_seq(&self) -> Option<u64> {
        self.checkpoint_anchor_seq.or_else(|| {
            if self.history_kind == RetainedBodyHistoryKind::YrsTextUpdateV1 {
                None
            } else {
                Some(self.seq)
            }
        })
    }

    pub(crate) fn to_public_revision(&self) -> Result<DocumentBodyRevision, StoreError> {
        Ok(self.retained_history()?.to_public_revision(
            self.seq,
            self.actor_id.clone(),
            self.document_id.clone(),
            self.checkpoint_anchor_seq,
            self.history_kind,
            self.timestamp_ms,
        ))
    }

    pub(crate) fn redacted(&self, exact_text: &str) -> Result<Option<Self>, StoreError> {
        let redacted = FULL_TEXT_BODY_MODEL
            .redact_history_payload(&self.retained_history()?, exact_text, "[REDACTED]")
            .map_err(StoreError::new)?;
        Ok(redacted.map(|payload| {
            let mut redacted = self.clone();
            redacted.history_kind = payload.kind();
            redacted.base_text = payload.base_text().to_owned();
            redacted.body_text = payload.storage_payload().to_owned();
            redacted.conflicted = payload.conflicted();
            redacted
        }))
    }

    pub(crate) fn redaction_match(
        &self,
        exact_text: &str,
    ) -> Result<Option<DocumentBodyRedactionMatch>, StoreError> {
        let Some(redacted) = self.redacted(exact_text)? else {
            return Ok(None);
        };
        let preview_lines = build_redaction_preview_lines(
            &self.retained_history()?,
            &redacted.retained_history()?,
            exact_text,
        );
        Ok(Some(DocumentBodyRedactionMatch {
            seq: self.seq,
            actor_id: self.actor_id.clone(),
            document_id: self.document_id.clone(),
            checkpoint_anchor_seq: self.checkpoint_anchor_seq,
            history_kind: self.history_kind.into(),
            occurrences: self.base_text.matches(exact_text).count()
                + self
                    .retained_history()?
                    .materialized_text()
                    .matches(exact_text)
                    .count(),
            preview_lines,
            timestamp_ms: self.timestamp_ms,
        }))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct FilePathRevision {
    pub seq: u64,
    #[serde(default)]
    pub workspace_cursor: u64,
    pub actor_id: String,
    pub document_id: String,
    pub mount_path: String,
    pub relative_path: String,
    pub deleted: bool,
    pub event_kind: String,
    pub timestamp_ms: u128,
}

pub(crate) fn parse_public_path_event_kind(raw: &str) -> Result<DocumentPathEventKind, StoreError> {
    DocumentPathEventKind::from_str(raw).map_err(StoreError::new)
}

pub(crate) fn effective_workspace_cursor(seq: u64, workspace_cursor: u64) -> u64 {
    if workspace_cursor == 0 {
        seq
    } else {
        workspace_cursor
    }
}

pub(crate) async fn insert_body_revision_tx(
    transaction: &tokio_postgres::Transaction<'_>,
    workspace_id: &str,
    document_id: &str,
    workspace_cursor: u64,
    actor_id: &str,
    checkpoint_anchor_seq: Option<u64>,
    payload: &RetainedBodyHistoryPayload,
) -> Result<(), StoreError> {
    transaction
        .execute(
            "insert into document_body_revisions \
             (workspace_id, document_id, workspace_cursor, actor_id, checkpoint_anchor_seq, history_kind, base_text, body_text, conflicted) \
             values ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            &[
                &workspace_id,
                &document_id,
                &(workspace_cursor as i64),
                &actor_id,
                &checkpoint_anchor_seq.map(|seq| seq as i64),
                &payload.kind().as_str(),
                &payload.base_text(),
                &payload.storage_payload(),
                &payload.conflicted(),
            ],
        )
        .await?;
    Ok(())
}

pub(crate) async fn insert_path_revision_tx(
    transaction: &tokio_postgres::Transaction<'_>,
    workspace_id: &str,
    document_id: &str,
    workspace_cursor: u64,
    actor_id: &str,
    mount_path: &str,
    relative_path: &str,
    deleted: bool,
    event_kind: &str,
) -> Result<(), StoreError> {
    transaction
        .execute(
            "insert into document_path_history \
             (workspace_id, document_id, workspace_cursor, actor_id, mount_path, relative_path, deleted, event_kind) \
             values ($1, $2, $3, $4, $5, $6, $7, $8)",
            &[
                &workspace_id,
                &document_id,
                &(workspace_cursor as i64),
                &actor_id,
                &mount_path,
                &relative_path,
                &deleted,
                &event_kind,
            ],
        )
        .await?;
    Ok(())
}

pub(crate) fn current_time_ms() -> Result<u128, StoreError> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .map_err(|err| StoreError::new(format!("current time before unix epoch: {err}")))
}
