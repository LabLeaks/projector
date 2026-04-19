/**
@module PROJECTOR.SERVER.HISTORY_COMPACTION
Owns retained-history policy resolution, checkpoint compaction, and checkpoint-plus-update replay for document body history.
*/
// @fileimplements PROJECTOR.SERVER.HISTORY_COMPACTION
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use projector_domain::{GetHistoryCompactionPolicyResponse, HistoryCompactionPolicy};
use serde::{Deserialize, Serialize};

use super::StoreError;
use super::body_state::{
    BodyStateModel, CanonicalBodyState, FULL_TEXT_BODY_MODEL, RetainedBodyHistoryKind,
};
use crate::storage::history::FileBodyRevision;

const DEFAULT_HISTORY_COMPACTION_REVISIONS: usize = 100;
const DEFAULT_HISTORY_COMPACTION_FREQUENCY: usize = 10;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct StoredHistoryCompactionPolicyOverride {
    pub(crate) repo_relative_path: PathBuf,
    pub(crate) policy: HistoryCompactionPolicy,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedHistoryCompactionPolicy {
    pub(crate) policy: HistoryCompactionPolicy,
    pub(crate) source_kind: &'static str,
    pub(crate) source_path: Option<PathBuf>,
}

fn default_history_compaction_policy() -> HistoryCompactionPolicy {
    HistoryCompactionPolicy {
        revisions: DEFAULT_HISTORY_COMPACTION_REVISIONS,
        frequency: DEFAULT_HISTORY_COMPACTION_FREQUENCY,
    }
}

pub(crate) fn resolve_history_compaction_policy(
    overrides: &[StoredHistoryCompactionPolicyOverride],
    repo_relative_path: &Path,
) -> ResolvedHistoryCompactionPolicy {
    let mut candidate = Some(repo_relative_path);
    while let Some(path) = candidate {
        if let Some(override_entry) = overrides
            .iter()
            .find(|entry| entry.repo_relative_path == path)
        {
            return ResolvedHistoryCompactionPolicy {
                policy: override_entry.policy.clone(),
                source_kind: if path == repo_relative_path {
                    "path_override"
                } else {
                    "ancestor_override"
                },
                source_path: Some(path.to_path_buf()),
            };
        }
        candidate = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty());
    }

    ResolvedHistoryCompactionPolicy {
        policy: default_history_compaction_policy(),
        source_kind: "default",
        source_path: None,
    }
}

pub(crate) fn history_compaction_response(
    overrides: &[StoredHistoryCompactionPolicyOverride],
    repo_relative_path: &Path,
) -> GetHistoryCompactionPolicyResponse {
    let resolved = resolve_history_compaction_policy(overrides, repo_relative_path);
    GetHistoryCompactionPolicyResponse {
        policy: resolved.policy,
        source_kind: resolved.source_kind.to_owned(),
        source_path: resolved.source_path.map(|path| path.display().to_string()),
    }
}

fn replay_document_revision_states(
    revisions: impl IntoIterator<Item = FileBodyRevision>,
) -> Vec<(FileBodyRevision, CanonicalBodyState)> {
    let mut states = HashMap::<String, CanonicalBodyState>::new();
    let mut anchored_states = HashMap::<(String, u64), CanonicalBodyState>::new();
    let mut replayed = Vec::new();
    for revision in revisions {
        let current_state = states.get(revision.document_id.as_str());
        let previous_state = if revision.history_kind == RetainedBodyHistoryKind::YrsTextUpdateV1 {
            current_state
                .filter(|state| state.materialized_text() == revision.base_text)
                .or_else(|| {
                    revision
                        .effective_checkpoint_anchor_seq()
                        .and_then(|anchor_seq| {
                            anchored_states.get(&(revision.document_id.clone(), anchor_seq))
                        })
                })
        } else {
            current_state
        };
        let next_state = revision.replayed_body_state(previous_state);
        if let Some(anchor_seq) = revision.effective_checkpoint_anchor_seq() {
            anchored_states.insert(
                (revision.document_id.clone(), anchor_seq),
                next_state.clone(),
            );
        }
        states.insert(revision.document_id.clone(), next_state.clone());
        replayed.push((revision, next_state));
    }
    replayed
}

fn replay_previous_state<'a>(
    revision: &FileBodyRevision,
    states: &'a HashMap<String, CanonicalBodyState>,
    anchored_states: &'a HashMap<(String, u64), CanonicalBodyState>,
) -> Option<&'a CanonicalBodyState> {
    let current_state = states.get(revision.document_id.as_str());
    if revision.history_kind != RetainedBodyHistoryKind::YrsTextUpdateV1 {
        return current_state;
    }
    current_state
        .filter(|state| state.materialized_text() == revision.base_text)
        .or_else(|| {
            revision
                .effective_checkpoint_anchor_seq()
                .and_then(|anchor_seq| {
                    anchored_states.get(&(revision.document_id.clone(), anchor_seq))
                })
        })
}

pub(crate) fn compact_document_body_revisions(
    revisions: &[FileBodyRevision],
    document_id: &str,
    policy: &HistoryCompactionPolicy,
) -> Result<Vec<FileBodyRevision>, StoreError> {
    let document_revisions = revisions
        .iter()
        .filter(|revision| revision.document_id == document_id)
        .cloned()
        .collect::<Vec<_>>();
    if document_revisions.len() <= policy.revisions {
        return Ok(document_revisions);
    }

    let older_len = document_revisions.len() - policy.revisions;
    let replayed = replay_document_revision_states(document_revisions);
    let mut compacted = Vec::new();
    let mut previous_kept_text = None::<String>;

    for (index, (revision, state)) in replayed.into_iter().enumerate() {
        let keep = index >= older_len || index % policy.frequency == 0;
        if !keep {
            continue;
        }

        let base_text = previous_kept_text
            .clone()
            .unwrap_or_else(|| revision.base_text.clone());
        let payload = FULL_TEXT_BODY_MODEL.checkpoint_history(base_text, state.materialized_text());
        let compacted_revision = FileBodyRevision {
            seq: revision.seq,
            workspace_cursor: revision.workspace_cursor,
            actor_id: revision.actor_id,
            document_id: revision.document_id,
            checkpoint_anchor_seq: Some(revision.seq),
            history_kind: payload.kind(),
            base_text: payload.base_text().to_owned(),
            body_text: payload.storage_payload().to_owned(),
            conflicted: payload.conflicted(),
            timestamp_ms: revision.timestamp_ms,
        };
        previous_kept_text = Some(state.materialized_text().to_owned());
        compacted.push(compacted_revision);
    }

    Ok(compacted)
}

pub(crate) fn replay_body_revision_run(
    revisions: impl IntoIterator<Item = FileBodyRevision>,
) -> HashMap<String, CanonicalBodyState> {
    let mut states = HashMap::<String, CanonicalBodyState>::new();
    let mut anchored_states = HashMap::<(String, u64), CanonicalBodyState>::new();
    for revision in revisions {
        let previous_state = replay_previous_state(&revision, &states, &anchored_states);
        let next_state = revision.replayed_body_state(previous_state);
        if let Some(anchor_seq) = revision.effective_checkpoint_anchor_seq() {
            anchored_states.insert(
                (revision.document_id.clone(), anchor_seq),
                next_state.clone(),
            );
        }
        states.insert(revision.document_id.clone(), next_state);
    }
    states
}

pub(crate) fn latest_checkpoint_anchor_seq(
    revisions: impl IntoIterator<Item = FileBodyRevision>,
    document_id: &str,
) -> Option<u64> {
    revisions
        .into_iter()
        .filter(|revision| revision.document_id == document_id)
        .filter_map(|revision| revision.effective_checkpoint_anchor_seq())
        .last()
}
