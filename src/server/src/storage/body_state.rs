use std::collections::hash_map::DefaultHasher;
/**
@module PROJECTOR.SERVER.BODY_STATE
Owns the internal canonical-body-state and retained-body-history abstractions so server storage can evolve beyond plain full-text revisions without changing current client-visible behavior yet.
*/
// @fileimplements PROJECTOR.SERVER.BODY_STATE
use std::hash::{Hash, Hasher};
use std::path::Path;

use projector_domain::{BootstrapSnapshot, DocumentBody, DocumentBodyRevision, DocumentId};
use serde::{Deserialize, Serialize};
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{Doc, GetString, ReadTxn, StateVector, Text, Transact, Update};

pub(crate) trait BodyStateModel {
    fn empty_state(&self) -> CanonicalBodyState;
    fn state_from_materialized_text(&self, text: impl Into<String>) -> CanonicalBodyState;
    fn state_from_yrs_checkpoint(
        &self,
        checkpoint: YrsTextCheckpoint,
    ) -> Result<CanonicalBodyState, String>;
    fn state_from_storage_record(
        &self,
        kind: CanonicalBodyStateKind,
        storage_payload: impl Into<String>,
    ) -> Result<CanonicalBodyState, String>;
    #[allow(dead_code)]
    fn history_from_stored_revision(
        &self,
        base_text: impl Into<String>,
        materialized_text: impl Into<String>,
        conflicted: bool,
    ) -> RetainedBodyHistoryPayload;
    fn checkpoint_history(
        &self,
        base_text: impl Into<String>,
        materialized_text: impl Into<String>,
    ) -> RetainedBodyHistoryPayload;
    fn history_from_storage_record(
        &self,
        kind: RetainedBodyHistoryKind,
        base_text: impl Into<String>,
        materialized_text: impl Into<String>,
        conflicted: bool,
    ) -> RetainedBodyHistoryPayload;
    fn redact_history_payload(
        &self,
        payload: &RetainedBodyHistoryPayload,
        exact_text: &str,
        replacement_text: &str,
    ) -> Result<Option<RetainedBodyHistoryPayload>, String>;
    fn created_history(&self, state: &CanonicalBodyState) -> RetainedBodyHistoryPayload;
    fn restored_history(
        &self,
        current_state: &CanonicalBodyState,
        target_state: &CanonicalBodyState,
    ) -> RetainedBodyHistoryPayload;
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct FullTextBodyModel;

pub(crate) const FULL_TEXT_BODY_MODEL: FullTextBodyModel = FullTextBodyModel;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CanonicalBodyStateKind {
    FullTextMergeV1,
    YrsTextCheckpointV1,
}

impl CanonicalBodyStateKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::FullTextMergeV1 => "full_text_merge_v1",
            Self::YrsTextCheckpointV1 => "yrs_text_checkpoint_v1",
        }
    }

    pub(crate) fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "full_text_merge_v1" => Ok(Self::FullTextMergeV1),
            "yrs_text_checkpoint_v1" => Ok(Self::YrsTextCheckpointV1),
            other => Err(format!("unknown canonical body state kind {other}")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CanonicalBodyState {
    kind: CanonicalBodyStateKind,
    storage_payload: String,
    materialized_text: String,
}

impl CanonicalBodyState {
    fn full_text_merge_v1(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            kind: CanonicalBodyStateKind::FullTextMergeV1,
            storage_payload: text.clone(),
            materialized_text: text,
        }
    }

    fn yrs_text_checkpoint_v1(checkpoint: YrsTextCheckpoint) -> Result<Self, String> {
        let materialized_text = checkpoint.materialized_text()?;
        Ok(Self {
            kind: CanonicalBodyStateKind::YrsTextCheckpointV1,
            storage_payload: encode_hex(checkpoint.checkpoint_bytes()),
            materialized_text,
        })
    }

    pub(crate) fn kind(&self) -> CanonicalBodyStateKind {
        self.kind
    }

    pub(crate) fn materialized_text(&self) -> &str {
        &self.materialized_text
    }

    pub(crate) fn storage_payload(&self) -> &str {
        &self.storage_payload
    }

    pub(crate) fn into_document_body(self, document_id: DocumentId) -> DocumentBody {
        DocumentBody {
            document_id,
            text: self.materialized_text,
        }
    }
}

pub(crate) fn body_state_from_snapshot(
    snapshot: &BootstrapSnapshot,
    document_id: &DocumentId,
) -> Option<CanonicalBodyState> {
    snapshot
        .bodies
        .iter()
        .find(|body| body.document_id == *document_id)
        .map(|body| FULL_TEXT_BODY_MODEL.state_from_materialized_text(body.text.clone()))
}

pub(crate) fn upsert_body_state(
    snapshot: &mut BootstrapSnapshot,
    document_id: &DocumentId,
    state: &CanonicalBodyState,
) {
    if let Some(body) = snapshot
        .bodies
        .iter_mut()
        .find(|body| body.document_id == *document_id)
    {
        body.text = state.materialized_text().to_owned();
    } else {
        snapshot
            .bodies
            .push(state.clone().into_document_body(document_id.clone()));
    }
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct YrsTextCheckpoint {
    merged_update_v1: Vec<u8>,
}

const PROJECTOR_YRS_SYNTHETIC_TEXT_CLIENT_ID: u64 = 1;

#[cfg_attr(not(test), allow(dead_code))]
impl YrsTextCheckpoint {
    pub(crate) fn from_materialized_text(text: &str) -> Result<Self, String> {
        let doc = Doc::with_client_id(PROJECTOR_YRS_SYNTHETIC_TEXT_CLIENT_ID);
        let body = doc.get_or_insert_text("body");
        let mut txn = doc.transact_mut();
        body.push(&mut txn, text);
        drop(txn);
        Self::from_doc(&doc)
    }

    pub(crate) fn from_doc(doc: &Doc) -> Result<Self, String> {
        let txn = doc.transact();
        let merged_update_v1 = txn.encode_diff_v1(&StateVector::default());
        let checkpoint = Self { merged_update_v1 };
        checkpoint.materialized_text()?;
        Ok(checkpoint)
    }

    pub(crate) fn from_checkpoint_bytes(merged_update_v1: Vec<u8>) -> Result<Self, String> {
        let checkpoint = Self { merged_update_v1 };
        checkpoint.materialized_text()?;
        Ok(checkpoint)
    }

    pub(crate) fn from_storage_payload(storage_payload: &str) -> Result<Self, String> {
        let merged_update_v1 = decode_hex(storage_payload)?;
        Self::from_checkpoint_bytes(merged_update_v1)
    }

    pub(crate) fn checkpoint_bytes(&self) -> &[u8] {
        &self.merged_update_v1
    }

    pub(crate) fn to_doc(&self) -> Result<Doc, String> {
        let doc = Doc::new();
        let update = Update::decode_v1(self.merged_update_v1.as_slice())
            .map_err(|err| format!("decode yrs checkpoint update: {err}"))?;
        let mut txn = doc.transact_mut();
        txn.apply_update(update)
            .map_err(|err| format!("apply yrs checkpoint update: {err}"))?;
        drop(txn);
        Ok(doc)
    }

    pub(crate) fn to_doc_with_client_id(&self, client_id: u64) -> Result<Doc, String> {
        let doc = Doc::with_client_id(client_id);
        let update = Update::decode_v1(self.merged_update_v1.as_slice())
            .map_err(|err| format!("decode yrs checkpoint update: {err}"))?;
        let mut txn = doc.transact_mut();
        txn.apply_update(update)
            .map_err(|err| format!("apply yrs checkpoint update: {err}"))?;
        drop(txn);
        Ok(doc)
    }

    pub(crate) fn materialized_text(&self) -> Result<String, String> {
        let doc = self.to_doc()?;
        let body = doc.get_or_insert_text("body");
        let txn = doc.transact();
        Ok(body.get_string(&txn))
    }

    pub(crate) fn state_vector_v1(&self) -> Result<Vec<u8>, String> {
        let doc = self.to_doc()?;
        let txn = doc.transact();
        Ok(txn.state_vector().encode_v1())
    }

    pub(crate) fn diff_update_v1(&self, remote_state_vector_v1: &[u8]) -> Result<Vec<u8>, String> {
        let doc = self.to_doc()?;
        let remote_state_vector = StateVector::decode_v1(remote_state_vector_v1)
            .map_err(|err| format!("decode yrs state vector: {err}"))?;
        let txn = doc.transact();
        Ok(txn.encode_diff_v1(&remote_state_vector))
    }

    pub(crate) fn with_update_v1(&self, update_v1: &[u8]) -> Result<Self, String> {
        let doc = self.to_doc()?;
        let update = Update::decode_v1(update_v1)
            .map_err(|err| format!("decode yrs incremental update: {err}"))?;
        let mut txn = doc.transact_mut();
        txn.apply_update(update)
            .map_err(|err| format!("apply yrs incremental update: {err}"))?;
        drop(txn);
        Self::from_doc(&doc)
    }

    pub(crate) fn update_to_text_v1(
        &self,
        next_text: &str,
        client_id: u64,
    ) -> Result<Vec<u8>, String> {
        let doc = self.to_doc_with_client_id(client_id)?;
        let current_text = self.materialized_text()?;
        let body = doc.get_or_insert_text("body");
        let mut txn = doc.transact_mut();
        let prefix = common_prefix_len(&current_text, next_text);
        let suffix = common_suffix_len(&current_text[prefix..], &next_text[prefix..]);
        let current_end = current_text.len().saturating_sub(suffix);
        let next_end = next_text.len().saturating_sub(suffix);
        let replace_start_chars = current_text[..prefix].chars().count() as u32;
        let replace_len_chars = current_text[prefix..current_end].chars().count() as u32;
        let inserted_len_chars = next_text[prefix..next_end].chars().count() as u32;
        let inserted_text = &next_text[prefix..next_end];
        if replace_len_chars > 0 && !inserted_text.is_empty() {
            body.insert(&mut txn, replace_start_chars, inserted_text);
            body.remove_range(
                &mut txn,
                replace_start_chars + inserted_len_chars,
                replace_len_chars,
            );
        } else {
            if replace_len_chars > 0 {
                body.remove_range(&mut txn, replace_start_chars, replace_len_chars);
            }
            if !inserted_text.is_empty() {
                body.insert(&mut txn, replace_start_chars, inserted_text);
            }
        }
        Ok(txn.encode_update_v1())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RetainedBodyHistoryKind {
    FullTextRevisionV1,
    FullTextCheckpointV1,
    YrsTextCheckpointV1,
    YrsTextUpdateV1,
}

impl RetainedBodyHistoryKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::FullTextRevisionV1 => "full_text_revision_v1",
            Self::FullTextCheckpointV1 => "full_text_checkpoint_v1",
            Self::YrsTextCheckpointV1 => "yrs_text_checkpoint_v1",
            Self::YrsTextUpdateV1 => "yrs_text_update_v1",
        }
    }

    pub(crate) fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "full_text_revision_v1" => Ok(Self::FullTextRevisionV1),
            "full_text_checkpoint_v1" => Ok(Self::FullTextCheckpointV1),
            "yrs_text_checkpoint_v1" => Ok(Self::YrsTextCheckpointV1),
            "yrs_text_update_v1" => Ok(Self::YrsTextUpdateV1),
            other => Err(format!("unknown retained body history kind {other}")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RetainedBodyHistoryPayload {
    kind: RetainedBodyHistoryKind,
    base_text: String,
    storage_payload: String,
    materialized_text: String,
    conflicted: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct StoredYrsTextUpdateV1 {
    update_v1_hex: String,
}

impl RetainedBodyHistoryPayload {
    fn full_text_revision_v1(
        base_text: impl Into<String>,
        materialized_text: impl Into<String>,
        conflicted: bool,
    ) -> Self {
        let materialized_text = materialized_text.into();
        Self {
            kind: RetainedBodyHistoryKind::FullTextRevisionV1,
            base_text: base_text.into(),
            storage_payload: materialized_text.clone(),
            materialized_text,
            conflicted,
        }
    }

    fn full_text_checkpoint_v1(
        base_text: impl Into<String>,
        materialized_text: impl Into<String>,
    ) -> Self {
        let materialized_text = materialized_text.into();
        Self {
            kind: RetainedBodyHistoryKind::FullTextCheckpointV1,
            base_text: base_text.into(),
            storage_payload: materialized_text.clone(),
            materialized_text,
            conflicted: false,
        }
    }

    fn yrs_text_checkpoint_v1(
        base_text: impl Into<String>,
        checkpoint: YrsTextCheckpoint,
    ) -> Result<Self, String> {
        let materialized_text = checkpoint.materialized_text()?;
        Ok(Self {
            kind: RetainedBodyHistoryKind::YrsTextCheckpointV1,
            base_text: base_text.into(),
            storage_payload: encode_hex(checkpoint.checkpoint_bytes()),
            materialized_text,
            conflicted: false,
        })
    }

    fn yrs_text_update_v1(
        base_text: impl Into<String>,
        update_v1: &[u8],
        materialized_text: impl Into<String>,
    ) -> Self {
        let materialized_text = materialized_text.into();
        Self {
            kind: RetainedBodyHistoryKind::YrsTextUpdateV1,
            base_text: base_text.into(),
            storage_payload: serde_json::to_string(&StoredYrsTextUpdateV1 {
                update_v1_hex: encode_hex(update_v1),
            })
            .expect("yrs text update payload should serialize"),
            materialized_text,
            conflicted: false,
        }
    }

    pub(crate) fn base_text(&self) -> &str {
        &self.base_text
    }

    pub(crate) fn kind(&self) -> RetainedBodyHistoryKind {
        self.kind
    }

    pub(crate) fn storage_payload(&self) -> &str {
        &self.storage_payload
    }

    pub(crate) fn materialized_text(&self) -> &str {
        &self.materialized_text
    }

    pub(crate) fn conflicted(&self) -> bool {
        self.conflicted
    }

    pub(crate) fn materialized_body_state(&self) -> CanonicalBodyState {
        FULL_TEXT_BODY_MODEL.state_from_materialized_text(self.materialized_text())
    }

    pub(crate) fn replayed_body_state(
        &self,
        previous_state: Option<&CanonicalBodyState>,
    ) -> CanonicalBodyState {
        FULL_TEXT_BODY_MODEL
            .replayed_history_state(self, previous_state)
            .unwrap_or_else(|_| self.materialized_body_state())
    }

    pub(crate) fn yrs_update_v1_bytes(&self) -> Result<Option<Vec<u8>>, String> {
        if self.kind != RetainedBodyHistoryKind::YrsTextUpdateV1 {
            return Ok(None);
        }
        let stored: StoredYrsTextUpdateV1 = serde_json::from_str(&self.storage_payload)
            .map_err(|err| format!("parse stored yrs update payload: {err}"))?;
        Ok(Some(decode_hex(&stored.update_v1_hex)?))
    }

    pub(crate) fn to_public_revision(
        &self,
        seq: u64,
        actor_id: String,
        document_id: String,
        checkpoint_anchor_seq: Option<u64>,
        history_kind: RetainedBodyHistoryKind,
        timestamp_ms: u128,
    ) -> DocumentBodyRevision {
        DocumentBodyRevision {
            seq,
            actor_id,
            document_id,
            checkpoint_anchor_seq,
            history_kind: history_kind.as_str().to_owned(),
            base_text: self.base_text.clone(),
            body_text: self.materialized_text.clone(),
            diff_lines: render_snapshot_diff_lines(&self.base_text, &self.materialized_text),
            conflicted: self.conflicted,
            timestamp_ms,
        }
    }
}

fn render_snapshot_diff_lines(base_text: &str, body_text: &str) -> Vec<String> {
    let base = split_lines_for_diff(base_text);
    let body = split_lines_for_diff(body_text);
    let lcs = build_lcs_table(&base, &body);
    let mut lines = vec![
        "--- base".to_owned(),
        "+++ snapshot".to_owned(),
        "@@".to_owned(),
    ];
    lines.extend(render_lcs_diff(&base, &body, &lcs, 0, 0));
    lines
}

fn split_lines_for_diff(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    text.split_inclusive('\n')
        .map(str::to_owned)
        .collect::<Vec<_>>()
}

fn build_lcs_table(left: &[String], right: &[String]) -> Vec<Vec<usize>> {
    let mut table = vec![vec![0; right.len() + 1]; left.len() + 1];
    for left_index in (0..left.len()).rev() {
        for right_index in (0..right.len()).rev() {
            table[left_index][right_index] = if left[left_index] == right[right_index] {
                table[left_index + 1][right_index + 1] + 1
            } else {
                table[left_index + 1][right_index].max(table[left_index][right_index + 1])
            };
        }
    }
    table
}

fn render_lcs_diff(
    left: &[String],
    right: &[String],
    lcs: &[Vec<usize>],
    mut left_index: usize,
    mut right_index: usize,
) -> Vec<String> {
    let mut lines = Vec::new();
    while left_index < left.len() && right_index < right.len() {
        if left[left_index] == right[right_index] {
            lines.push(format!(" {}", left[left_index].trim_end_matches('\n')));
            left_index += 1;
            right_index += 1;
        } else if lcs[left_index + 1][right_index] >= lcs[left_index][right_index + 1] {
            lines.push(format!("-{}", left[left_index].trim_end_matches('\n')));
            left_index += 1;
        } else {
            lines.push(format!("+{}", right[right_index].trim_end_matches('\n')));
            right_index += 1;
        }
    }
    while left_index < left.len() {
        lines.push(format!("-{}", left[left_index].trim_end_matches('\n')));
        left_index += 1;
    }
    while right_index < right.len() {
        lines.push(format!("+{}", right[right_index].trim_end_matches('\n')));
        right_index += 1;
    }
    lines
}

impl BodyStateModel for FullTextBodyModel {
    fn empty_state(&self) -> CanonicalBodyState {
        self.state_from_materialized_text(String::new())
    }

    fn state_from_materialized_text(&self, text: impl Into<String>) -> CanonicalBodyState {
        let text = text.into();
        self.state_from_yrs_checkpoint(
            YrsTextCheckpoint::from_materialized_text(&text)
                .expect("materialized UTF-8 text should convert into a yrs checkpoint"),
        )
        .expect("yrs checkpoint should materialize into canonical text state")
    }

    fn state_from_yrs_checkpoint(
        &self,
        checkpoint: YrsTextCheckpoint,
    ) -> Result<CanonicalBodyState, String> {
        CanonicalBodyState::yrs_text_checkpoint_v1(checkpoint)
    }

    fn state_from_storage_record(
        &self,
        kind: CanonicalBodyStateKind,
        storage_payload: impl Into<String>,
    ) -> Result<CanonicalBodyState, String> {
        let storage_payload = storage_payload.into();
        match kind {
            CanonicalBodyStateKind::FullTextMergeV1 => {
                Ok(CanonicalBodyState::full_text_merge_v1(storage_payload))
            }
            CanonicalBodyStateKind::YrsTextCheckpointV1 => self.state_from_yrs_checkpoint(
                YrsTextCheckpoint::from_storage_payload(&storage_payload)?,
            ),
        }
    }

    fn history_from_stored_revision(
        &self,
        base_text: impl Into<String>,
        materialized_text: impl Into<String>,
        conflicted: bool,
    ) -> RetainedBodyHistoryPayload {
        let base_text = base_text.into();
        let materialized_text = materialized_text.into();
        if conflicted {
            RetainedBodyHistoryPayload::full_text_revision_v1(base_text, materialized_text, true)
        } else {
            let update_v1 = YrsTextCheckpoint::from_materialized_text(&base_text)
                .expect("base text should convert into a yrs checkpoint")
                .update_to_text_v1(&materialized_text, PROJECTOR_YRS_SYNTHETIC_TEXT_CLIENT_ID)
                .expect("next text should produce a yrs incremental update");
            RetainedBodyHistoryPayload::yrs_text_update_v1(base_text, &update_v1, materialized_text)
        }
    }

    fn checkpoint_history(
        &self,
        base_text: impl Into<String>,
        materialized_text: impl Into<String>,
    ) -> RetainedBodyHistoryPayload {
        let base_text = base_text.into();
        let materialized_text = materialized_text.into();
        RetainedBodyHistoryPayload::yrs_text_checkpoint_v1(
            base_text,
            YrsTextCheckpoint::from_materialized_text(&materialized_text)
                .expect("materialized UTF-8 text should convert into a yrs checkpoint"),
        )
        .expect("yrs checkpoint should materialize into retained history payload")
    }

    fn history_from_storage_record(
        &self,
        kind: RetainedBodyHistoryKind,
        base_text: impl Into<String>,
        materialized_text: impl Into<String>,
        conflicted: bool,
    ) -> RetainedBodyHistoryPayload {
        match kind {
            RetainedBodyHistoryKind::FullTextRevisionV1 => {
                RetainedBodyHistoryPayload::full_text_revision_v1(
                    base_text,
                    materialized_text,
                    conflicted,
                )
            }
            RetainedBodyHistoryKind::FullTextCheckpointV1 => {
                RetainedBodyHistoryPayload::full_text_checkpoint_v1(base_text, materialized_text)
            }
            RetainedBodyHistoryKind::YrsTextCheckpointV1 => {
                let storage_payload = materialized_text.into();
                if storage_payload.is_empty() {
                    return RetainedBodyHistoryPayload::full_text_checkpoint_v1(base_text, "");
                }
                RetainedBodyHistoryPayload::yrs_text_checkpoint_v1(
                    base_text,
                    YrsTextCheckpoint::from_storage_payload(&storage_payload)
                        .expect("stored yrs history payload should decode"),
                )
                .expect("stored yrs history payload should materialize")
            }
            RetainedBodyHistoryKind::YrsTextUpdateV1 => {
                let base_text = base_text.into();
                let storage_payload = materialized_text.into();
                if storage_payload.is_empty() {
                    return RetainedBodyHistoryPayload::full_text_checkpoint_v1(base_text, "");
                }
                let stored: StoredYrsTextUpdateV1 = serde_json::from_str(&storage_payload)
                    .expect("stored yrs update payload should parse");
                let update_v1 = decode_hex(&stored.update_v1_hex)
                    .expect("stored yrs update payload should decode");
                let materialized_text = YrsTextCheckpoint::from_materialized_text(&base_text)
                    .expect("base text should convert into a yrs checkpoint")
                    .with_update_v1(&update_v1)
                    .expect("stored yrs update payload should apply")
                    .materialized_text()
                    .expect("stored yrs update payload should materialize");
                RetainedBodyHistoryPayload::yrs_text_update_v1(
                    base_text,
                    &update_v1,
                    materialized_text,
                )
            }
        }
    }

    fn redact_history_payload(
        &self,
        payload: &RetainedBodyHistoryPayload,
        exact_text: &str,
        replacement_text: &str,
    ) -> Result<Option<RetainedBodyHistoryPayload>, String> {
        if exact_text.is_empty() {
            return Err("exact redaction text must not be empty".to_owned());
        }

        let redacted_base = payload.base_text().replace(exact_text, replacement_text);
        let redacted_body = payload
            .materialized_text()
            .replace(exact_text, replacement_text);
        if redacted_base == payload.base_text() && redacted_body == payload.materialized_text() {
            return Ok(None);
        }

        let redacted = match payload.kind() {
            RetainedBodyHistoryKind::FullTextRevisionV1 => {
                RetainedBodyHistoryPayload::full_text_revision_v1(
                    redacted_base,
                    redacted_body,
                    payload.conflicted(),
                )
            }
            RetainedBodyHistoryKind::FullTextCheckpointV1 => {
                RetainedBodyHistoryPayload::full_text_checkpoint_v1(redacted_base, redacted_body)
            }
            RetainedBodyHistoryKind::YrsTextCheckpointV1 => {
                self.checkpoint_history(redacted_base, redacted_body)
            }
            RetainedBodyHistoryKind::YrsTextUpdateV1 => {
                self.history_from_stored_revision(redacted_base, redacted_body, false)
            }
        };
        Ok(Some(redacted))
    }

    fn created_history(&self, state: &CanonicalBodyState) -> RetainedBodyHistoryPayload {
        self.checkpoint_history("", state.materialized_text())
    }

    fn restored_history(
        &self,
        current_state: &CanonicalBodyState,
        target_state: &CanonicalBodyState,
    ) -> RetainedBodyHistoryPayload {
        self.checkpoint_history(
            current_state.materialized_text(),
            target_state.materialized_text(),
        )
    }
}

impl FullTextBodyModel {
    fn yrs_update_v1_from_payload(&self, storage_payload: &str) -> Result<Vec<u8>, String> {
        let stored: StoredYrsTextUpdateV1 = serde_json::from_str(storage_payload)
            .map_err(|err| format!("parse stored yrs update payload: {err}"))?;
        decode_hex(&stored.update_v1_hex)
    }

    fn yrs_checkpoint_from_state(
        &self,
        state: &CanonicalBodyState,
    ) -> Result<YrsTextCheckpoint, String> {
        match state.kind() {
            CanonicalBodyStateKind::FullTextMergeV1 => {
                YrsTextCheckpoint::from_materialized_text(state.materialized_text())
            }
            CanonicalBodyStateKind::YrsTextCheckpointV1 => {
                YrsTextCheckpoint::from_storage_payload(state.storage_payload())
            }
        }
    }

    fn replayed_history_state(
        &self,
        payload: &RetainedBodyHistoryPayload,
        previous_state: Option<&CanonicalBodyState>,
    ) -> Result<CanonicalBodyState, String> {
        if payload.kind() != RetainedBodyHistoryKind::YrsTextUpdateV1 {
            return Ok(payload.materialized_body_state());
        }

        let Some(previous_state) = previous_state else {
            return Ok(payload.materialized_body_state());
        };

        let update_v1 = self.yrs_update_v1_from_payload(payload.storage_payload())?;
        let checkpoint = self
            .yrs_checkpoint_from_state(previous_state)?
            .with_update_v1(&update_v1)?;
        self.state_from_yrs_checkpoint(checkpoint)
    }
}

pub(crate) trait BodyConvergenceEngine {
    fn apply_update(
        &self,
        actor_id: &str,
        base_text: &str,
        current_state: &CanonicalBodyState,
        incoming_text: &str,
    ) -> BodyConvergenceResult;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BodyConvergenceResult {
    canonical_state: CanonicalBodyState,
    retained_history: RetainedBodyHistoryPayload,
    concurrent: bool,
}

impl BodyConvergenceResult {
    pub(crate) fn canonical_state(&self) -> &CanonicalBodyState {
        &self.canonical_state
    }

    pub(crate) fn retained_history(&self) -> &RetainedBodyHistoryPayload {
        &self.retained_history
    }

    pub(crate) fn summary_for_path(&self, mount_path: &Path, relative_path: &Path) -> String {
        let display_path = if relative_path.as_os_str().is_empty() {
            mount_path.display().to_string()
        } else {
            mount_path.join(relative_path).display().to_string()
        };
        if self.retained_history.conflicted() {
            return format!(
                "merged conflicting text update at {display_path} with conflict markers"
            );
        }
        if self.concurrent {
            return format!("merged concurrent text update at {display_path}");
        }
        format!("updated text document at {display_path}")
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct YrsConvergenceBodyEngine;

impl BodyConvergenceEngine for YrsConvergenceBodyEngine {
    fn apply_update(
        &self,
        actor_id: &str,
        base_text: &str,
        current_state: &CanonicalBodyState,
        incoming_text: &str,
    ) -> BodyConvergenceResult {
        let current_text = current_state.materialized_text();
        let concurrent = current_text != base_text;
        let current_checkpoint = FULL_TEXT_BODY_MODEL
            .yrs_checkpoint_from_state(current_state)
            .expect("current canonical state should decode into a yrs checkpoint");
        let base_checkpoint = if concurrent {
            YrsTextCheckpoint::from_materialized_text(base_text)
                .expect("base text should convert into a yrs checkpoint")
        } else {
            current_checkpoint.clone()
        };
        let incoming_update = base_checkpoint
            .update_to_text_v1(
                incoming_text,
                yrs_operation_client_id(actor_id, base_text, incoming_text),
            )
            .expect("incoming text should produce a yrs incremental update");
        let canonical_checkpoint = current_checkpoint
            .with_update_v1(&incoming_update)
            .expect("yrs update should merge into current canonical state");
        let canonical_state = FULL_TEXT_BODY_MODEL
            .state_from_yrs_checkpoint(canonical_checkpoint)
            .expect("merged yrs checkpoint should materialize as canonical text state");
        BodyConvergenceResult {
            retained_history: RetainedBodyHistoryPayload::yrs_text_update_v1(
                base_text,
                &incoming_update,
                canonical_state.materialized_text(),
            ),
            canonical_state,
            concurrent,
        }
    }
}

fn common_prefix_len(left: &str, right: &str) -> usize {
    let mut total = 0;
    for (left_char, right_char) in left.chars().zip(right.chars()) {
        if left_char != right_char {
            break;
        }
        total += left_char.len_utf8();
    }
    total
}

fn common_suffix_len(left: &str, right: &str) -> usize {
    let mut total = 0;
    for (left_char, right_char) in left.chars().rev().zip(right.chars().rev()) {
        if left_char != right_char {
            break;
        }
        total += left_char.len_utf8();
        if total >= left.len() || total >= right.len() {
            break;
        }
    }
    total.min(left.len()).min(right.len())
}

fn yrs_operation_client_id(actor_id: &str, base_text: &str, incoming_text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    actor_id.hash(&mut hasher);
    base_text.hash(&mut hasher);
    incoming_text.hash(&mut hasher);
    let hashed = hasher.finish();
    if hashed == PROJECTOR_YRS_SYNTHETIC_TEXT_CLIENT_ID {
        hashed.wrapping_add(1)
    } else {
        hashed
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut encoded, "{byte:02x}");
    }
    encoded
}

fn decode_hex(raw: &str) -> Result<Vec<u8>, String> {
    if raw.len() % 2 != 0 {
        return Err("hex payload must have even length".to_owned());
    }
    let mut decoded = Vec::with_capacity(raw.len() / 2);
    let bytes = raw.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let chunk = std::str::from_utf8(&bytes[index..index + 2])
            .map_err(|err| format!("invalid utf8 in hex payload: {err}"))?;
        let byte = u8::from_str_radix(chunk, 16)
            .map_err(|err| format!("invalid hex byte `{chunk}`: {err}"))?;
        decoded.push(byte);
        index += 2;
    }
    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use super::{
        BodyConvergenceEngine, BodyStateModel, CanonicalBodyStateKind, FullTextBodyModel,
        RetainedBodyHistoryKind, RetainedBodyHistoryPayload, YrsConvergenceBodyEngine,
        YrsTextCheckpoint,
    };
    use yrs::{GetString, Text, Transact};

    #[test]
    fn yrs_engine_merges_non_overlapping_edits() {
        let engine = YrsConvergenceBodyEngine;
        let base_text = "alpha\nbeta\ngamma\n";
        let base_state = FullTextBodyModel.state_from_materialized_text(base_text);
        let current = engine.apply_update(
            "actor-a",
            base_text,
            &base_state,
            "alpha\nbeta changed\ngamma\n",
        );
        let result = engine.apply_update(
            "actor-b",
            base_text,
            current.canonical_state(),
            "alpha\nbeta\nGAMMA changed\n",
        );

        assert_eq!(
            result.canonical_state().materialized_text(),
            "alpha\nbeta changed\nGAMMA changed\n"
        );
        assert!(!result.retained_history().conflicted());
    }

    #[test]
    fn yrs_engine_applies_sequential_updates_without_losing_text() {
        let engine = YrsConvergenceBodyEngine;
        let base_text = "<p>original revision</p>\n";
        let base_state = FullTextBodyModel.state_from_materialized_text(base_text);
        let current = engine.apply_update(
            "projector",
            base_text,
            &base_state,
            "<p>updated revision one</p>\n",
        );
        let result = engine.apply_update(
            "projector",
            "<p>updated revision one</p>\n",
            current.canonical_state(),
            "<p>updated revision two</p>\n",
        );

        assert_eq!(
            result.canonical_state().materialized_text(),
            "<p>updated revision two</p>\n"
        );
        assert!(!result.retained_history().conflicted());
    }

    #[test]
    fn yrs_engine_converges_overlapping_edits_without_conflict_markers() {
        let engine = YrsConvergenceBodyEngine;
        let base_text = "alpha\nbeta\ngamma\n";
        let base_state = FullTextBodyModel.state_from_materialized_text(base_text);
        let current =
            engine.apply_update("actor-a", base_text, &base_state, "alpha\nBETA\ngamma\n");
        let result = engine.apply_update(
            "actor-b",
            base_text,
            current.canonical_state(),
            "alpha\nBETTER\ngamma\n",
        );

        assert!(!result.retained_history().conflicted());
        let merged = result.canonical_state().materialized_text();
        assert!(!merged.contains("<<<<<<<"));
        assert!(merged.contains("BETA") || merged.contains("BETTER"));
    }

    #[test]
    fn checkpoint_history_round_trips_as_distinct_kind() {
        let model = FullTextBodyModel;
        let payload = model.checkpoint_history("before\n", "after\n");

        assert_eq!(payload.kind(), RetainedBodyHistoryKind::YrsTextCheckpointV1);
        assert_eq!(payload.base_text(), "before\n");
        assert_eq!(payload.materialized_text(), "after\n");
        assert_ne!(payload.storage_payload(), "after\n");
        assert!(!payload.conflicted());

        let decoded = model.history_from_storage_record(
            RetainedBodyHistoryKind::YrsTextCheckpointV1,
            payload.base_text(),
            payload.storage_payload(),
            true,
        );
        assert_eq!(decoded.kind(), RetainedBodyHistoryKind::YrsTextCheckpointV1);
        assert_eq!(decoded.base_text(), "before\n");
        assert_eq!(decoded.materialized_text(), "after\n");
        assert_eq!(decoded.storage_payload(), payload.storage_payload());
        assert!(!decoded.conflicted());
    }

    #[test]
    fn non_conflicted_history_defaults_to_yrs_update_payload() {
        let model = FullTextBodyModel;
        let payload = model.history_from_stored_revision("before\n", "after\n", false);

        assert_eq!(payload.kind(), RetainedBodyHistoryKind::YrsTextUpdateV1);
        assert_eq!(payload.base_text(), "before\n");
        assert_eq!(payload.materialized_text(), "after\n");
        assert!(!payload.conflicted());
        assert_ne!(payload.storage_payload(), "after\n");
        assert!(!payload.storage_payload().contains("after\n"));
    }

    #[test]
    fn yrs_update_history_round_trips_against_base_text() {
        let model = FullTextBodyModel;
        let payload = model.history_from_stored_revision("before\n", "after\n", false);

        let decoded = model.history_from_storage_record(
            RetainedBodyHistoryKind::YrsTextUpdateV1,
            payload.base_text(),
            payload.storage_payload(),
            false,
        );
        assert_eq!(decoded.kind(), RetainedBodyHistoryKind::YrsTextUpdateV1);
        assert_eq!(decoded.base_text(), "before\n");
        assert_eq!(decoded.materialized_text(), "after\n");
        assert_eq!(decoded.storage_payload(), payload.storage_payload());
        assert!(!decoded.conflicted());
    }

    #[test]
    fn yrs_update_history_replays_against_prior_state_when_available() {
        let model = FullTextBodyModel;
        let prior_state = model.state_from_materialized_text("before\n");
        let payload = model.history_from_stored_revision("before\n", "after\n", false);
        let replayed = RetainedBodyHistoryPayload {
            materialized_text: "stale cached text\n".to_owned(),
            ..payload
        }
        .replayed_body_state(Some(&prior_state));

        assert_eq!(replayed.kind(), CanonicalBodyStateKind::YrsTextCheckpointV1);
        assert_eq!(replayed.materialized_text(), "after\n");
    }

    #[test]
    fn purged_yrs_history_payloads_decode_as_empty_retained_history() {
        let model = FullTextBodyModel;
        let checkpoint = model.history_from_storage_record(
            RetainedBodyHistoryKind::YrsTextCheckpointV1,
            "",
            "",
            false,
        );
        assert_eq!(checkpoint.base_text(), "");
        assert_eq!(checkpoint.materialized_text(), "");

        let update = model.history_from_storage_record(
            RetainedBodyHistoryKind::YrsTextUpdateV1,
            "",
            "",
            false,
        );
        assert_eq!(update.base_text(), "");
        assert_eq!(update.materialized_text(), "");
    }

    #[test]
    fn redact_history_payload_rewrites_yrs_history_without_losing_kind() {
        let model = FullTextBodyModel;
        let payload =
            model.history_from_stored_revision("before SECRET\n", "after SECRET\n", false);

        let redacted = model
            .redact_history_payload(&payload, "SECRET", "[REDACTED]")
            .expect("redact retained payload")
            .expect("retained payload should change");

        assert_eq!(redacted.kind(), RetainedBodyHistoryKind::YrsTextUpdateV1);
        assert_eq!(redacted.base_text(), "before [REDACTED]\n");
        assert_eq!(redacted.materialized_text(), "after [REDACTED]\n");
    }

    #[test]
    fn created_and_restored_history_use_checkpoint_payloads() {
        let model = FullTextBodyModel;
        let created = model.created_history(&model.state_from_materialized_text("created\n"));
        let restored = model.restored_history(
            &model.state_from_materialized_text("before\n"),
            &model.state_from_materialized_text("after\n"),
        );

        assert_eq!(created.kind(), RetainedBodyHistoryKind::YrsTextCheckpointV1);
        assert_eq!(
            restored.kind(),
            RetainedBodyHistoryKind::YrsTextCheckpointV1
        );
    }

    #[test]
    fn conflicted_history_stays_on_full_text_revision_kind() {
        let model = FullTextBodyModel;
        let payload = model.history_from_stored_revision(
            "before\n",
            "<<<<<<< existing\na\n=======\nb\n>>>>>>> incoming\n",
            true,
        );

        assert_eq!(payload.kind(), RetainedBodyHistoryKind::FullTextRevisionV1);
        assert!(payload.conflicted());
        assert_eq!(payload.storage_payload(), payload.materialized_text());
    }

    #[test]
    fn materialized_text_defaults_to_yrs_canonical_state() {
        let model = FullTextBodyModel;
        let state = model.state_from_materialized_text("default yrs body\n");

        assert_eq!(state.kind(), CanonicalBodyStateKind::YrsTextCheckpointV1);
        assert_eq!(state.materialized_text(), "default yrs body\n");
        assert_ne!(state.storage_payload(), "default yrs body\n");
    }

    #[test]
    fn yrs_checkpoint_materializes_text_and_round_trips_checkpoint_bytes() {
        let checkpoint = YrsTextCheckpoint::from_materialized_text("hello from yrs\n")
            .expect("build checkpoint");

        assert_eq!(
            checkpoint
                .materialized_text()
                .expect("materialize checkpoint"),
            "hello from yrs\n"
        );

        let reloaded =
            YrsTextCheckpoint::from_checkpoint_bytes(checkpoint.checkpoint_bytes().to_vec())
                .expect("reload checkpoint");
        assert_eq!(
            reloaded
                .materialized_text()
                .expect("materialize reloaded checkpoint"),
            "hello from yrs\n"
        );
    }

    #[test]
    fn yrs_checkpoint_converges_concurrent_updates_via_exchanged_updates() {
        let base = YrsTextCheckpoint::from_materialized_text("alpha\nbeta\ngamma\n")
            .expect("base checkpoint");
        let local_doc = base.to_doc().expect("load local doc");
        let remote_doc = base.to_doc().expect("load remote doc");

        {
            let text = local_doc.get_or_insert_text("body");
            let mut txn = local_doc.transact_mut();
            text.insert(&mut txn, 6, "local ");
        }
        {
            let text = remote_doc.get_or_insert_text("body");
            let mut txn = remote_doc.transact_mut();
            text.insert(&mut txn, 17, "remote ");
        }

        let local = YrsTextCheckpoint::from_doc(&local_doc).expect("checkpoint local");
        let remote = YrsTextCheckpoint::from_doc(&remote_doc).expect("checkpoint remote");
        let local_update = local
            .diff_update_v1(&remote.state_vector_v1().expect("remote state vector"))
            .expect("local diff update");
        let remote_update = remote
            .diff_update_v1(&local.state_vector_v1().expect("local state vector"))
            .expect("remote diff update");

        let merged_local = local
            .with_update_v1(&remote_update)
            .expect("merge remote into local");
        let merged_remote = remote
            .with_update_v1(&local_update)
            .expect("merge local into remote");
        let expected = "alpha\nlocal beta\ngamma\nremote ";

        assert_eq!(
            merged_local
                .materialized_text()
                .expect("materialize merged local"),
            expected
        );
        assert_eq!(
            merged_remote
                .materialized_text()
                .expect("materialize merged remote"),
            expected
        );
        assert!(!expected.contains("<<<<<<<"));
    }

    #[test]
    fn yrs_checkpoint_rebuilds_doc_from_checkpoint_bytes() {
        let checkpoint = YrsTextCheckpoint::from_materialized_text("rebuilt from checkpoint")
            .expect("checkpoint");
        let rebuilt_doc = checkpoint.to_doc().expect("rebuild doc");
        let text = rebuilt_doc.get_or_insert_text("body");
        let txn = rebuilt_doc.transact();

        assert_eq!(text.get_string(&txn), "rebuilt from checkpoint");
    }

    #[test]
    fn yrs_canonical_state_round_trips_through_storage_payload() {
        let model = FullTextBodyModel;
        let checkpoint =
            YrsTextCheckpoint::from_materialized_text("canonical yrs text\n").expect("checkpoint");
        let state = model
            .state_from_yrs_checkpoint(checkpoint)
            .expect("yrs canonical state");

        assert_eq!(state.kind(), CanonicalBodyStateKind::YrsTextCheckpointV1);
        assert_eq!(state.materialized_text(), "canonical yrs text\n");
        assert_ne!(state.storage_payload(), "canonical yrs text\n");

        let decoded = model
            .state_from_storage_record(
                CanonicalBodyStateKind::YrsTextCheckpointV1,
                state.storage_payload(),
            )
            .expect("decode stored yrs payload");
        assert_eq!(decoded.kind(), CanonicalBodyStateKind::YrsTextCheckpointV1);
        assert_eq!(decoded.materialized_text(), "canonical yrs text\n");
        assert_eq!(decoded.storage_payload(), state.storage_payload());
    }
}
