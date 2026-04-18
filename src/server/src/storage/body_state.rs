/**
@module PROJECTOR.SERVER.BODY_STATE
Owns the internal canonical-body-state and retained-body-history abstractions so server storage can evolve beyond plain full-text revisions without changing current client-visible behavior yet.
*/
// @fileimplements PROJECTOR.SERVER.BODY_STATE
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

#[cfg_attr(not(test), allow(dead_code))]
impl YrsTextCheckpoint {
    pub(crate) fn from_materialized_text(text: &str) -> Result<Self, String> {
        let doc = Doc::new();
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RetainedBodyHistoryKind {
    FullTextRevisionV1,
    FullTextCheckpointV1,
    YrsTextCheckpointV1,
}

impl RetainedBodyHistoryKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::FullTextRevisionV1 => "full_text_revision_v1",
            Self::FullTextCheckpointV1 => "full_text_checkpoint_v1",
            Self::YrsTextCheckpointV1 => "yrs_text_checkpoint_v1",
        }
    }

    pub(crate) fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "full_text_revision_v1" => Ok(Self::FullTextRevisionV1),
            "full_text_checkpoint_v1" => Ok(Self::FullTextCheckpointV1),
            "yrs_text_checkpoint_v1" => Ok(Self::YrsTextCheckpointV1),
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

    pub(crate) fn to_public_revision(
        &self,
        seq: u64,
        actor_id: String,
        document_id: String,
        timestamp_ms: u128,
    ) -> DocumentBodyRevision {
        DocumentBodyRevision {
            seq,
            actor_id,
            document_id,
            base_text: self.base_text.clone(),
            body_text: self.materialized_text.clone(),
            conflicted: self.conflicted,
            timestamp_ms,
        }
    }
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
        RetainedBodyHistoryPayload::full_text_revision_v1(base_text, materialized_text, conflicted)
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
                self.history_from_stored_revision(base_text, materialized_text, conflicted)
            }
            RetainedBodyHistoryKind::FullTextCheckpointV1 => {
                RetainedBodyHistoryPayload::full_text_checkpoint_v1(base_text, materialized_text)
            }
            RetainedBodyHistoryKind::YrsTextCheckpointV1 => {
                let storage_payload = materialized_text.into();
                RetainedBodyHistoryPayload::yrs_text_checkpoint_v1(
                    base_text,
                    YrsTextCheckpoint::from_storage_payload(&storage_payload)
                        .expect("stored yrs history payload should decode"),
                )
                .expect("stored yrs history payload should materialize")
            }
        }
    }

    fn created_history(&self, state: &CanonicalBodyState) -> RetainedBodyHistoryPayload {
        self.history_from_stored_revision("", state.materialized_text(), false)
    }

    fn restored_history(
        &self,
        current_state: &CanonicalBodyState,
        target_state: &CanonicalBodyState,
    ) -> RetainedBodyHistoryPayload {
        self.history_from_stored_revision(
            current_state.materialized_text(),
            target_state.materialized_text(),
            false,
        )
    }
}

pub(crate) trait BodyConvergenceEngine {
    fn apply_update(
        &self,
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
pub(crate) struct ThreeWayMergeBodyEngine;

impl BodyConvergenceEngine for ThreeWayMergeBodyEngine {
    fn apply_update(
        &self,
        base_text: &str,
        current_state: &CanonicalBodyState,
        incoming_text: &str,
    ) -> BodyConvergenceResult {
        let current_text = current_state.materialized_text();

        if current_text == base_text {
            let canonical_state = FULL_TEXT_BODY_MODEL.state_from_materialized_text(incoming_text);
            return BodyConvergenceResult {
                retained_history: FULL_TEXT_BODY_MODEL.history_from_stored_revision(
                    base_text,
                    canonical_state.materialized_text(),
                    false,
                ),
                canonical_state,
                concurrent: false,
            };
        }

        if incoming_text == base_text || incoming_text == current_text {
            let canonical_state = current_state.clone();
            return BodyConvergenceResult {
                retained_history: FULL_TEXT_BODY_MODEL.history_from_stored_revision(
                    base_text,
                    canonical_state.materialized_text(),
                    false,
                ),
                canonical_state,
                concurrent: true,
            };
        }

        let current_span = change_span(base_text, current_text);
        let incoming_span = change_span(base_text, incoming_text);

        if ranges_do_not_overlap(&current_span, &incoming_span) {
            let merged = apply_non_overlapping_replacements(
                base_text,
                current_text,
                incoming_text,
                &current_span,
                &incoming_span,
            );
            let canonical_state = FULL_TEXT_BODY_MODEL.state_from_materialized_text(merged);
            return BodyConvergenceResult {
                retained_history: FULL_TEXT_BODY_MODEL.history_from_stored_revision(
                    base_text,
                    canonical_state.materialized_text(),
                    false,
                ),
                canonical_state,
                concurrent: true,
            };
        }

        let conflicted = format!(
            "<<<<<<< existing\n{}=======\n{}>>>>>>> incoming\n",
            ensure_trailing_newline(current_text),
            ensure_trailing_newline(incoming_text),
        );
        let canonical_state = FULL_TEXT_BODY_MODEL.state_from_materialized_text(conflicted);
        BodyConvergenceResult {
            retained_history: FULL_TEXT_BODY_MODEL.history_from_stored_revision(
                base_text,
                canonical_state.materialized_text(),
                true,
            ),
            canonical_state,
            concurrent: true,
        }
    }
}

#[derive(Clone, Copy)]
struct ChangeSpan {
    start: usize,
    end: usize,
}

fn change_span(base: &str, variant: &str) -> ChangeSpan {
    let prefix = common_prefix_len(base, variant);
    let suffix = common_suffix_len(&base[prefix..], &variant[prefix..]);
    ChangeSpan {
        start: prefix,
        end: base.len().saturating_sub(suffix),
    }
}

fn ranges_do_not_overlap(left: &ChangeSpan, right: &ChangeSpan) -> bool {
    left.end <= right.start || right.end <= left.start
}

fn apply_non_overlapping_replacements(
    base: &str,
    current: &str,
    incoming: &str,
    current_span: &ChangeSpan,
    incoming_span: &ChangeSpan,
) -> String {
    let (first_span, first_variant, second_span, second_variant) =
        if current_span.start <= incoming_span.start {
            (current_span, current, incoming_span, incoming)
        } else {
            (incoming_span, incoming, current_span, current)
        };

    let mut merged = String::new();
    merged.push_str(&base[..first_span.start]);
    merged.push_str(&first_variant[first_span.start..variant_end(first_variant, base, first_span)]);
    merged.push_str(&base[first_span.end..second_span.start]);
    merged.push_str(
        &second_variant[second_span.start..variant_end(second_variant, base, second_span)],
    );
    merged.push_str(&base[second_span.end..]);
    merged
}

fn variant_end(variant: &str, base: &str, span: &ChangeSpan) -> usize {
    let prefix = span.start;
    let suffix = common_suffix_len(&base[prefix..], &variant[prefix..]);
    variant.len().saturating_sub(suffix)
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

fn ensure_trailing_newline(text: &str) -> String {
    if text.ends_with('\n') {
        text.to_owned()
    } else {
        format!("{text}\n")
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
        BodyConvergenceEngine, BodyStateModel, CanonicalBodyState, CanonicalBodyStateKind,
        FullTextBodyModel, RetainedBodyHistoryKind, ThreeWayMergeBodyEngine, YrsTextCheckpoint,
    };
    use yrs::{GetString, Text, Transact};

    #[test]
    fn three_way_engine_merges_non_overlapping_edits() {
        let engine = ThreeWayMergeBodyEngine;
        let current_state = CanonicalBodyState::full_text_merge_v1("alpha\nbeta changed\ngamma\n");
        let result = engine.apply_update(
            "alpha\nbeta\ngamma\n",
            &current_state,
            "alpha\nbeta\nGAMMA changed\n",
        );

        assert_eq!(
            result.canonical_state().materialized_text(),
            "alpha\nbeta changed\nGAMMA changed\n"
        );
        assert!(!result.retained_history().conflicted());
    }

    #[test]
    fn three_way_engine_emits_conflict_markers_for_overlapping_edits() {
        let engine = ThreeWayMergeBodyEngine;
        let current_state = CanonicalBodyState::full_text_merge_v1("alpha\nBETA\ngamma\n");
        let result = engine.apply_update(
            "alpha\nbeta\ngamma\n",
            &current_state,
            "alpha\nBETTER\ngamma\n",
        );

        assert!(result.retained_history().conflicted());
        assert!(
            result
                .canonical_state()
                .materialized_text()
                .contains("<<<<<<< existing")
        );
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
