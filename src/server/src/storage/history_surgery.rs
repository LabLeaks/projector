/**
@module PROJECTOR.SERVER.HISTORY_SURGERY
Owns retained-history match derivation, preview rendering, stale-preview guards, and non-secret audit summaries for redact and purge flows.
*/
// @fileimplements PROJECTOR.SERVER.HISTORY_SURGERY
use projector_domain::{DocumentBodyPurgeMatch, DocumentBodyRedactionMatch};

use super::StoreError;
use super::body_state::RetainedBodyHistoryPayload;
use crate::storage::history::FileBodyRevision;

const HISTORY_REDACTION_MARKER: &str = "[REDACTED]";

pub(crate) fn build_redaction_preview_lines(
    original: &RetainedBodyHistoryPayload,
    redacted: &RetainedBodyHistoryPayload,
    exact_text: &str,
) -> Vec<String> {
    let mut lines = collect_redaction_preview_lines(original.materialized_text(), exact_text);
    if !original.base_text().is_empty() {
        lines.extend(collect_redaction_preview_lines(
            original.base_text(),
            exact_text,
        ));
    }
    if lines.is_empty() {
        let redacted_lines =
            collect_redaction_preview_lines(redacted.materialized_text(), HISTORY_REDACTION_MARKER);
        lines.extend(redacted_lines);
    }
    lines.truncate(8);
    lines
}

fn collect_redaction_preview_lines(text: &str, needle: &str) -> Vec<String> {
    text.lines()
        .filter(|line| line.contains(needle))
        .flat_map(|line| {
            let trimmed = line.trim();
            let redacted = trimmed.replace(needle, HISTORY_REDACTION_MARKER);
            [format!("- {trimmed}"), format!("+ {redacted}")]
        })
        .collect()
}

pub(crate) fn retained_redaction_matches(
    revisions: impl IntoIterator<Item = FileBodyRevision>,
    document_id: &str,
    exact_text: &str,
    limit: usize,
) -> Result<Vec<DocumentBodyRedactionMatch>, StoreError> {
    let mut matches = revisions
        .into_iter()
        .filter(|revision| revision.document_id == document_id)
        .map(|revision| revision.redaction_match(exact_text))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    if matches.len() > limit {
        matches = matches.split_off(matches.len() - limit);
    }
    Ok(matches)
}

pub(crate) fn retained_purge_matches(
    revisions: impl IntoIterator<Item = FileBodyRevision>,
    document_id: &str,
    limit: usize,
) -> Vec<DocumentBodyPurgeMatch> {
    let mut matches = revisions
        .into_iter()
        .filter(|revision| revision.document_id == document_id)
        .filter(|revision| !revision.base_text.is_empty() || !revision.body_text.is_empty())
        .map(|revision| DocumentBodyPurgeMatch {
            seq: revision.seq,
            actor_id: revision.actor_id,
            document_id: revision.document_id,
            checkpoint_anchor_seq: revision.checkpoint_anchor_seq,
            history_kind: revision.history_kind.into(),
            body_len: revision.body_text.len(),
            timestamp_ms: revision.timestamp_ms,
        })
        .collect::<Vec<_>>();
    if matches.len() > limit {
        matches = matches.split_off(matches.len() - limit);
    }
    matches
}

pub(crate) fn ensure_expected_history_match_set(
    document_id: &str,
    expected_match_seqs: Option<&Vec<u64>>,
    matched_seqs: &[u64],
    operation_name: &str,
) -> Result<(), StoreError> {
    let Some(expected_match_seqs) = expected_match_seqs else {
        return Ok(());
    };
    if expected_match_seqs == matched_seqs {
        return Ok(());
    }
    Err(StoreError::new(format!(
        "document {} retained {} preview is stale: expected seqs {:?}, found {:?}",
        document_id, operation_name, expected_match_seqs, matched_seqs
    )))
}

pub(crate) fn purge_history_summary(
    document_id: &str,
    mount_relative_path: Option<&str>,
    relative_path: Option<&str>,
) -> String {
    match (mount_relative_path, relative_path) {
        (Some(mount), Some(relative)) if !relative.is_empty() => {
            format!("purged retained body history for {mount}/{relative}")
        }
        (Some(mount), _) => format!("purged retained body history for {mount}"),
        _ => format!("purged retained body history for document {document_id}"),
    }
}

pub(crate) fn redact_history_summary(
    document_id: &str,
    mount_relative_path: Option<&str>,
    relative_path: Option<&str>,
) -> String {
    match (mount_relative_path, relative_path) {
        (Some(mount), Some(relative)) if !relative.is_empty() => {
            format!("redacted retained body history for {mount}/{relative}")
        }
        (Some(mount), _) => format!("redacted retained body history for {mount}"),
        _ => format!("redacted retained body history for document {document_id}"),
    }
}
