/**
@module PROJECTOR.SERVER.HISTORY
Owns durable document body-revision and path-revision capture for file-backed and Postgres-backed stores so future restore workflows have explicit history beyond current state and provenance summaries.
*/
// @fileimplements PROJECTOR.SERVER.HISTORY
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio_postgres::{Client, GenericClient};

use super::StoreError;
use super::bodies::{file_persist_workspace_snapshot, file_read_workspace_snapshot};
use super::body_persistence::{
    AsyncBodyPersistence, FileBodyPersistence, PostgresBodyPersistence, SnapshotBodyPersistence,
};
use super::body_projection::{snapshot_from_current_rows, snapshot_from_manifest_entries};
use super::body_state::{
    BodyStateModel, CanonicalBodyState, FULL_TEXT_BODY_MODEL, RetainedBodyHistoryKind,
    RetainedBodyHistoryPayload,
};
use super::history_compaction::{
    StoredHistoryCompactionPolicyOverride, compact_document_body_revisions,
    history_compaction_response, replay_body_revision_run, resolve_history_compaction_policy,
};
use super::history_restore::{
    build_restored_live_workspace_snapshot, diff_workspace_restore_changes,
};
use super::history_surgery::{
    build_redaction_preview_lines, ensure_expected_history_match_set, purge_history_summary,
    redact_history_summary, retained_purge_matches, retained_redaction_matches,
};
use super::provenance::{
    current_workspace_cursor_tx, file_append_workspace_event, file_workspace_cursor,
    insert_event_tx,
};
use super::workspaces::workspace_dir;
use projector_domain::{
    BootstrapSnapshot, ClearHistoryCompactionPolicyRequest, DocumentBodyPurgeMatch,
    DocumentBodyRedactionMatch, DocumentBodyRevision, DocumentId, DocumentKind,
    DocumentPathRevision, GetHistoryCompactionPolicyResponse, HistoryCompactionPolicy,
    ManifestEntry, PreviewPurgeDocumentBodyHistoryRequest, PreviewRedactDocumentBodyHistoryRequest,
    ProvenanceEvent, ProvenanceEventKind, PurgeDocumentBodyHistoryRequest,
    RedactDocumentBodyHistoryRequest, RestoreWorkspaceRequest, SetHistoryCompactionPolicyRequest,
};

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

    pub(crate) fn retained_history(&self) -> RetainedBodyHistoryPayload {
        FULL_TEXT_BODY_MODEL.history_from_storage_record(
            self.history_kind,
            self.base_text.clone(),
            self.body_text.clone(),
            self.conflicted,
        )
    }

    pub(crate) fn materialized_body_state(&self) -> CanonicalBodyState {
        self.retained_history().materialized_body_state()
    }

    pub(crate) fn replayed_body_state(
        &self,
        previous_state: Option<&CanonicalBodyState>,
    ) -> CanonicalBodyState {
        self.retained_history().replayed_body_state(previous_state)
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

    pub(crate) fn to_public_revision(&self) -> DocumentBodyRevision {
        self.retained_history().to_public_revision(
            self.seq,
            self.actor_id.clone(),
            self.document_id.clone(),
            self.checkpoint_anchor_seq,
            self.history_kind,
            self.timestamp_ms,
        )
    }

    pub(crate) fn redacted(&self, exact_text: &str) -> Result<Option<Self>, StoreError> {
        let redacted = FULL_TEXT_BODY_MODEL
            .redact_history_payload(&self.retained_history(), exact_text, "[REDACTED]")
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
            &self.retained_history(),
            &redacted.retained_history(),
            exact_text,
        );
        Ok(Some(DocumentBodyRedactionMatch {
            seq: self.seq,
            actor_id: self.actor_id.clone(),
            document_id: self.document_id.clone(),
            checkpoint_anchor_seq: self.checkpoint_anchor_seq,
            history_kind: self.history_kind.as_str().to_owned(),
            occurrences: self.base_text.matches(exact_text).count()
                + self
                    .retained_history()
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

fn file_body_revisions_path(state_dir: &Path, workspace_id: &str) -> std::path::PathBuf {
    workspace_dir(state_dir, workspace_id).join("body_revisions.json")
}

fn file_history_compaction_policies_path(
    state_dir: &Path,
    workspace_id: &str,
) -> std::path::PathBuf {
    workspace_dir(state_dir, workspace_id).join("history_compaction_policies.json")
}

fn effective_workspace_cursor(seq: u64, workspace_cursor: u64) -> u64 {
    if workspace_cursor == 0 {
        seq
    } else {
        workspace_cursor
    }
}

fn file_path_history_path(state_dir: &Path, workspace_id: &str) -> std::path::PathBuf {
    workspace_dir(state_dir, workspace_id).join("path_history.json")
}

pub(crate) fn file_read_body_revisions(
    state_dir: &Path,
    workspace_id: &str,
) -> Result<Vec<FileBodyRevision>, StoreError> {
    let path = file_body_revisions_path(state_dir, workspace_id);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read(path)?;
    serde_json::from_slice(&content).map_err(|err| StoreError::new(err.to_string()))
}

pub(crate) fn file_append_body_revision(
    state_dir: &Path,
    workspace_id: &str,
    revision: FileBodyRevision,
) -> Result<(), StoreError> {
    let workspace_root = workspace_dir(state_dir, workspace_id);
    fs::create_dir_all(&workspace_root)?;
    let path = file_body_revisions_path(state_dir, workspace_id);
    let mut revisions = file_read_body_revisions(state_dir, workspace_id)?;
    revisions.push(revision);
    let encoded =
        serde_json::to_vec_pretty(&revisions).map_err(|err| StoreError::new(err.to_string()))?;
    fs::write(path, encoded)?;
    Ok(())
}

fn file_write_body_revisions(
    state_dir: &Path,
    workspace_id: &str,
    revisions: &[FileBodyRevision],
) -> Result<(), StoreError> {
    let workspace_root = workspace_dir(state_dir, workspace_id);
    fs::create_dir_all(&workspace_root)?;
    let encoded =
        serde_json::to_vec_pretty(revisions).map_err(|err| StoreError::new(err.to_string()))?;
    fs::write(file_body_revisions_path(state_dir, workspace_id), encoded)?;
    Ok(())
}

fn file_read_history_compaction_policies(
    state_dir: &Path,
    workspace_id: &str,
) -> Result<Vec<StoredHistoryCompactionPolicyOverride>, StoreError> {
    let path = file_history_compaction_policies_path(state_dir, workspace_id);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read(path)?;
    serde_json::from_slice(&content).map_err(|err| StoreError::new(err.to_string()))
}

fn file_write_history_compaction_policies(
    state_dir: &Path,
    workspace_id: &str,
    overrides: &[StoredHistoryCompactionPolicyOverride],
) -> Result<(), StoreError> {
    let workspace_root = workspace_dir(state_dir, workspace_id);
    fs::create_dir_all(&workspace_root)?;
    let encoded =
        serde_json::to_vec_pretty(overrides).map_err(|err| StoreError::new(err.to_string()))?;
    fs::write(
        file_history_compaction_policies_path(state_dir, workspace_id),
        encoded,
    )?;
    Ok(())
}

pub(crate) fn file_get_history_compaction_policy(
    state_dir: &Path,
    workspace_id: &str,
    repo_relative_path: &str,
) -> Result<GetHistoryCompactionPolicyResponse, StoreError> {
    Ok(history_compaction_response(
        &file_read_history_compaction_policies(state_dir, workspace_id)?,
        Path::new(repo_relative_path),
    ))
}

pub(crate) fn file_set_history_compaction_policy(
    state_dir: &Path,
    request: &SetHistoryCompactionPolicyRequest,
) -> Result<(), StoreError> {
    let mut overrides = file_read_history_compaction_policies(state_dir, &request.workspace_id)?;
    let repo_relative_path = PathBuf::from(&request.repo_relative_path);
    if let Some(existing) = overrides
        .iter_mut()
        .find(|entry| entry.repo_relative_path == repo_relative_path)
    {
        existing.policy = request.policy.clone();
    } else {
        overrides.push(StoredHistoryCompactionPolicyOverride {
            repo_relative_path,
            policy: request.policy.clone(),
        });
    }
    overrides.sort_by(|left, right| left.repo_relative_path.cmp(&right.repo_relative_path));
    file_write_history_compaction_policies(state_dir, &request.workspace_id, &overrides)
}

pub(crate) fn file_clear_history_compaction_policy(
    state_dir: &Path,
    request: &ClearHistoryCompactionPolicyRequest,
) -> Result<bool, StoreError> {
    let mut overrides = file_read_history_compaction_policies(state_dir, &request.workspace_id)?;
    let original_len = overrides.len();
    overrides.retain(|entry| entry.repo_relative_path != Path::new(&request.repo_relative_path));
    let removed = overrides.len() != original_len;
    if removed {
        file_write_history_compaction_policies(state_dir, &request.workspace_id, &overrides)?;
    }
    Ok(removed)
}

pub(crate) fn file_enforce_history_compaction_policy(
    state_dir: &Path,
    workspace_id: &str,
    document_id: &str,
) -> Result<(), StoreError> {
    let snapshot = file_read_workspace_snapshot(state_dir, workspace_id)?;
    let Some(entry) = snapshot
        .manifest
        .entries
        .iter()
        .find(|entry| !entry.deleted && entry.document_id.as_str() == document_id)
    else {
        return Ok(());
    };
    let repo_relative_path = entry.mount_relative_path.join(&entry.relative_path);
    let resolved = resolve_history_compaction_policy(
        &file_read_history_compaction_policies(state_dir, workspace_id)?,
        &repo_relative_path,
    );
    let revisions = file_read_body_revisions(state_dir, workspace_id)?;
    let compacted = compact_document_body_revisions(&revisions, document_id, &resolved.policy)?;
    let original = revisions
        .iter()
        .filter(|revision| revision.document_id == document_id)
        .cloned()
        .collect::<Vec<_>>();
    if compacted == original {
        return Ok(());
    }
    let mut rewritten = revisions
        .into_iter()
        .filter(|revision| revision.document_id != document_id)
        .collect::<Vec<_>>();
    rewritten.extend(compacted);
    rewritten.sort_by_key(|revision| revision.seq);
    file_write_body_revisions(state_dir, workspace_id, &rewritten)
}

pub(crate) fn file_list_body_revisions(
    state_dir: &Path,
    workspace_id: &str,
    document_id: &str,
    limit: usize,
) -> Result<Vec<DocumentBodyRevision>, StoreError> {
    let mut revisions = file_read_body_revisions(state_dir, workspace_id)?
        .into_iter()
        .filter(|revision| revision.document_id == document_id)
        .map(|revision| revision.to_public_revision())
        .collect::<Vec<_>>();
    if revisions.len() > limit {
        revisions = revisions.split_off(revisions.len() - limit);
    }
    Ok(revisions)
}

pub(crate) fn file_preview_redact_document_body_history(
    state_dir: &Path,
    request: &PreviewRedactDocumentBodyHistoryRequest,
) -> Result<Vec<DocumentBodyRedactionMatch>, StoreError> {
    let matches = retained_redaction_matches(
        file_read_body_revisions(state_dir, &request.workspace_id)?,
        &request.document_id,
        &request.exact_text,
        request.limit,
    )?;
    if matches.is_empty() {
        return Err(StoreError::new(format!(
            "document {} has no retained body history matching {:?} in workspace {}",
            request.document_id, request.exact_text, request.workspace_id
        )));
    }
    Ok(matches)
}

pub(crate) fn file_preview_purge_document_body_history(
    state_dir: &Path,
    request: &PreviewPurgeDocumentBodyHistoryRequest,
) -> Result<Vec<DocumentBodyPurgeMatch>, StoreError> {
    let matches = retained_purge_matches(
        file_read_body_revisions(state_dir, &request.workspace_id)?,
        &request.document_id,
        request.limit,
    );
    if matches.is_empty() {
        return Err(StoreError::new(format!(
            "document {} has no retained body history in workspace {}",
            request.document_id, request.workspace_id
        )));
    }
    Ok(matches)
}

pub(crate) fn file_purge_document_body_history(
    state_dir: &Path,
    request: &PurgeDocumentBodyHistoryRequest,
) -> Result<(), StoreError> {
    let mut revisions = file_read_body_revisions(state_dir, &request.workspace_id)?;
    let mut matched_seqs = Vec::new();
    for revision in revisions.iter_mut() {
        if revision.document_id == request.document_id
            && (!revision.base_text.is_empty() || !revision.body_text.is_empty())
        {
            matched_seqs.push(revision.seq);
            revision.base_text.clear();
            revision.body_text.clear();
        }
    }
    if matched_seqs.is_empty() {
        return Err(StoreError::new(format!(
            "document {} has no retained body history in workspace {}",
            request.document_id, request.workspace_id
        )));
    }
    ensure_expected_history_match_set(
        &request.document_id,
        request.expected_match_seqs.as_ref(),
        &matched_seqs,
        "purge",
    )?;
    let encoded =
        serde_json::to_vec_pretty(&revisions).map_err(|err| StoreError::new(err.to_string()))?;
    fs::write(
        file_body_revisions_path(state_dir, &request.workspace_id),
        encoded,
    )?;

    let current_snapshot = file_read_workspace_snapshot(state_dir, &request.workspace_id)?;
    let live_entry = current_snapshot
        .manifest
        .entries
        .iter()
        .find(|entry| !entry.deleted && entry.document_id.as_str() == request.document_id);
    let mount_relative_path =
        live_entry.map(|entry| entry.mount_relative_path.display().to_string());
    let relative_path = live_entry.map(|entry| entry.relative_path.display().to_string());
    let event_cursor = file_workspace_cursor(state_dir, &request.workspace_id)? + 1;
    file_append_workspace_event(
        state_dir,
        &request.workspace_id,
        ProvenanceEvent {
            cursor: event_cursor,
            timestamp_ms: current_time_ms(),
            actor_id: projector_domain::ActorId::new(request.actor_id.clone()),
            document_id: Some(DocumentId::new(request.document_id.clone())),
            mount_relative_path: mount_relative_path.clone(),
            relative_path: relative_path.clone(),
            summary: purge_history_summary(
                request.document_id.as_str(),
                mount_relative_path.as_deref(),
                relative_path.as_deref(),
            ),
            kind: ProvenanceEventKind::DocumentHistoryPurged,
        },
    )?;
    Ok(())
}

pub(crate) fn file_redact_document_body_history(
    state_dir: &Path,
    request: &RedactDocumentBodyHistoryRequest,
) -> Result<(), StoreError> {
    let mut revisions = file_read_body_revisions(state_dir, &request.workspace_id)?;
    let mut matched_seqs = Vec::new();
    for revision in revisions.iter_mut() {
        if revision.document_id != request.document_id {
            continue;
        }
        if let Some(redacted) = revision.redacted(&request.exact_text)? {
            matched_seqs.push(redacted.seq);
            *revision = redacted;
        }
    }
    if matched_seqs.is_empty() {
        return Err(StoreError::new(format!(
            "document {} has no retained body history matching {:?} in workspace {}",
            request.document_id, request.exact_text, request.workspace_id
        )));
    }
    ensure_expected_history_match_set(
        &request.document_id,
        request.expected_match_seqs.as_ref(),
        &matched_seqs,
        "redaction",
    )?;
    let encoded =
        serde_json::to_vec_pretty(&revisions).map_err(|err| StoreError::new(err.to_string()))?;
    fs::write(
        file_body_revisions_path(state_dir, &request.workspace_id),
        encoded,
    )?;

    let current_snapshot = file_read_workspace_snapshot(state_dir, &request.workspace_id)?;
    let live_entry = current_snapshot
        .manifest
        .entries
        .iter()
        .find(|entry| !entry.deleted && entry.document_id.as_str() == request.document_id);
    let mount_relative_path =
        live_entry.map(|entry| entry.mount_relative_path.display().to_string());
    let relative_path = live_entry.map(|entry| entry.relative_path.display().to_string());
    let event_cursor = file_workspace_cursor(state_dir, &request.workspace_id)? + 1;
    file_append_workspace_event(
        state_dir,
        &request.workspace_id,
        ProvenanceEvent {
            cursor: event_cursor,
            timestamp_ms: current_time_ms(),
            actor_id: projector_domain::ActorId::new(request.actor_id.clone()),
            document_id: Some(DocumentId::new(request.document_id.clone())),
            mount_relative_path: mount_relative_path.clone(),
            relative_path: relative_path.clone(),
            summary: redact_history_summary(
                request.document_id.as_str(),
                mount_relative_path.as_deref(),
                relative_path.as_deref(),
            ),
            kind: ProvenanceEventKind::DocumentHistoryRedacted,
        },
    )?;
    Ok(())
}

pub(crate) fn file_read_path_history(
    state_dir: &Path,
    workspace_id: &str,
) -> Result<Vec<FilePathRevision>, StoreError> {
    let path = file_path_history_path(state_dir, workspace_id);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read(path)?;
    serde_json::from_slice(&content).map_err(|err| StoreError::new(err.to_string()))
}

pub(crate) fn file_append_path_revision(
    state_dir: &Path,
    workspace_id: &str,
    revision: FilePathRevision,
) -> Result<(), StoreError> {
    let workspace_root = workspace_dir(state_dir, workspace_id);
    fs::create_dir_all(&workspace_root)?;
    let path = file_path_history_path(state_dir, workspace_id);
    let mut revisions = file_read_path_history(state_dir, workspace_id)?;
    revisions.push(revision);
    let encoded =
        serde_json::to_vec_pretty(&revisions).map_err(|err| StoreError::new(err.to_string()))?;
    fs::write(path, encoded)?;
    Ok(())
}

pub(crate) fn file_list_path_revisions(
    state_dir: &Path,
    workspace_id: &str,
    document_id: &str,
    limit: usize,
) -> Result<Vec<DocumentPathRevision>, StoreError> {
    let mut revisions = file_read_path_history(state_dir, workspace_id)?
        .into_iter()
        .filter(|revision| revision.document_id == document_id)
        .map(|revision| DocumentPathRevision {
            seq: revision.seq,
            actor_id: revision.actor_id,
            document_id: revision.document_id,
            mount_path: revision.mount_path,
            relative_path: revision.relative_path,
            deleted: revision.deleted,
            event_kind: revision.event_kind,
            timestamp_ms: revision.timestamp_ms,
        })
        .collect::<Vec<_>>();
    if revisions.len() > limit {
        revisions = revisions.split_off(revisions.len() - limit);
    }
    Ok(revisions)
}

pub(crate) fn file_resolve_document_by_historical_path(
    state_dir: &Path,
    workspace_id: &str,
    mount_path: &str,
    relative_path: &str,
) -> Result<DocumentId, StoreError> {
    file_read_path_history(state_dir, workspace_id)?
        .into_iter()
        .filter(|revision| {
            revision.mount_path == mount_path && revision.relative_path == relative_path
        })
        .max_by_key(|revision| revision.seq)
        .map(|revision| DocumentId::new(revision.document_id))
        .ok_or_else(|| {
            StoreError::new(format!(
                "no document path history found at {mount_path}/{relative_path}"
            ))
        })
}

pub(crate) fn file_reconstruct_workspace_at_cursor(
    state_dir: &Path,
    workspace_id: &str,
    cursor: u64,
) -> Result<BootstrapSnapshot, StoreError> {
    let path_history = file_read_path_history(state_dir, workspace_id)?;
    let body_history = file_read_body_revisions(state_dir, workspace_id)?;

    let latest_paths = path_history
        .into_iter()
        .filter(|revision| {
            effective_workspace_cursor(revision.seq, revision.workspace_cursor) <= cursor
        })
        .fold(
            HashMap::<String, FilePathRevision>::new(),
            |mut acc, revision| {
                let replace = acc
                    .get(&revision.document_id)
                    .map(|current| {
                        effective_workspace_cursor(revision.seq, revision.workspace_cursor)
                            > effective_workspace_cursor(current.seq, current.workspace_cursor)
                            || (effective_workspace_cursor(revision.seq, revision.workspace_cursor)
                                == effective_workspace_cursor(
                                    current.seq,
                                    current.workspace_cursor,
                                )
                                && revision.seq > current.seq)
                    })
                    .unwrap_or(true);
                if replace {
                    acc.insert(revision.document_id.clone(), revision);
                }
                acc
            },
        );

    let latest_bodies = replay_body_revision_run(body_history.into_iter().filter(|revision| {
        effective_workspace_cursor(revision.seq, revision.workspace_cursor) <= cursor
    }));

    let mut entries = latest_paths
        .into_values()
        .map(|revision| ManifestEntry {
            document_id: DocumentId::new(revision.document_id),
            mount_relative_path: revision.mount_path.into(),
            relative_path: revision.relative_path.into(),
            kind: DocumentKind::Text,
            deleted: revision.deleted,
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        left.mount_relative_path
            .cmp(&right.mount_relative_path)
            .then_with(|| left.relative_path.cmp(&right.relative_path))
            .then_with(|| left.document_id.as_str().cmp(right.document_id.as_str()))
    });

    Ok(snapshot_from_manifest_entries(entries, |document_id| {
        latest_bodies.get(document_id.as_str()).cloned()
    }))
}

pub(crate) fn file_restore_workspace_at_cursor(
    state_dir: &Path,
    request: &RestoreWorkspaceRequest,
) -> Result<(), StoreError> {
    let current_cursor = file_workspace_cursor(state_dir, &request.workspace_id)?;
    let provided_cursor = request
        .based_on_cursor
        .ok_or_else(|| StoreError::new("workspace restore missing based_on_cursor precondition"))?;
    if provided_cursor != current_cursor {
        return Err(StoreError::conflict(
            "stale_cursor",
            format!(
                "workspace restore based on stale cursor {provided_cursor}; current workspace cursor is {current_cursor}"
            ),
        ));
    }
    if request.cursor > current_cursor {
        return Err(StoreError::new(format!(
            "workspace restore target cursor {} is newer than current workspace cursor {}",
            request.cursor, current_cursor
        )));
    }

    let current_snapshot = file_read_workspace_snapshot(state_dir, &request.workspace_id)?;
    let target_snapshot =
        file_reconstruct_workspace_at_cursor(state_dir, &request.workspace_id, request.cursor)?;
    let restored_snapshot =
        build_restored_live_workspace_snapshot(&current_snapshot, &target_snapshot);
    let changes =
        diff_workspace_restore_changes(&current_snapshot, &restored_snapshot, request.cursor);

    let body_persistence = FileBodyPersistence::new(state_dir, &request.workspace_id);
    file_persist_workspace_snapshot(state_dir, &request.workspace_id, &restored_snapshot)?;
    for change in changes {
        let event_cursor = file_workspace_cursor(state_dir, &request.workspace_id)? + 1;
        file_append_workspace_event(
            state_dir,
            &request.workspace_id,
            ProvenanceEvent {
                cursor: event_cursor,
                timestamp_ms: current_time_ms(),
                actor_id: projector_domain::ActorId::new(request.actor_id.clone()),
                document_id: Some(change.document_id.clone()),
                mount_relative_path: Some(change.path.mount_path.clone()),
                relative_path: Some(change.path.relative_path.clone()),
                summary: change.summary.clone(),
                kind: change.kind.clone(),
            },
        )?;
        if let Some(body) = change.body {
            body_persistence.append_retained_history(
                event_cursor,
                &request.actor_id,
                change.document_id.as_str(),
                &FULL_TEXT_BODY_MODEL.checkpoint_history(body.base_text, body.body_text),
                current_time_ms(),
            )?;
        }
        file_append_path_revision(
            state_dir,
            &request.workspace_id,
            FilePathRevision {
                seq: event_cursor,
                workspace_cursor: event_cursor,
                actor_id: request.actor_id.clone(),
                document_id: change.document_id.as_str().to_owned(),
                mount_path: change.path.mount_path,
                relative_path: change.path.relative_path,
                deleted: change.path.deleted,
                event_kind: change.path.event_kind,
                timestamp_ms: current_time_ms(),
            },
        )?;
    }

    Ok(())
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

async fn postgres_read_history_compaction_policies(
    client: &impl GenericClient,
    workspace_id: &str,
) -> Result<Vec<StoredHistoryCompactionPolicyOverride>, StoreError> {
    let rows = client
        .query(
            "select repo_relative_path, revisions, frequency \
             from history_compaction_policies \
             where workspace_id = $1 \
             order by repo_relative_path asc",
            &[&workspace_id],
        )
        .await?;
    Ok(rows
        .into_iter()
        .map(|row| StoredHistoryCompactionPolicyOverride {
            repo_relative_path: PathBuf::from(row.get::<_, String>("repo_relative_path")),
            policy: HistoryCompactionPolicy {
                revisions: row.get::<_, i32>("revisions") as usize,
                frequency: row.get::<_, i32>("frequency") as usize,
            },
        })
        .collect())
}

pub(crate) async fn postgres_get_history_compaction_policy(
    client: &Client,
    workspace_id: &str,
    repo_relative_path: &str,
) -> Result<GetHistoryCompactionPolicyResponse, StoreError> {
    Ok(history_compaction_response(
        &postgres_read_history_compaction_policies(client, workspace_id).await?,
        Path::new(repo_relative_path),
    ))
}

pub(crate) async fn postgres_set_history_compaction_policy(
    transaction: &tokio_postgres::Transaction<'_>,
    request: &SetHistoryCompactionPolicyRequest,
) -> Result<(), StoreError> {
    transaction
        .execute(
            "insert into history_compaction_policies (workspace_id, repo_relative_path, revisions, frequency) \
             values ($1, $2, $3, $4) \
             on conflict (workspace_id, repo_relative_path) do update set \
               revisions = excluded.revisions, \
               frequency = excluded.frequency",
            &[
                &request.workspace_id,
                &request.repo_relative_path,
                &(request.policy.revisions as i32),
                &(request.policy.frequency as i32),
            ],
        )
        .await?;
    Ok(())
}

pub(crate) async fn postgres_clear_history_compaction_policy(
    transaction: &tokio_postgres::Transaction<'_>,
    request: &ClearHistoryCompactionPolicyRequest,
) -> Result<bool, StoreError> {
    let removed = transaction
        .execute(
            "delete from history_compaction_policies \
             where workspace_id = $1 and repo_relative_path = $2",
            &[&request.workspace_id, &request.repo_relative_path],
        )
        .await?;
    Ok(removed > 0)
}

pub(crate) async fn postgres_list_body_revisions(
    client: &Client,
    workspace_id: &str,
    document_id: &str,
    limit: usize,
) -> Result<Vec<DocumentBodyRevision>, StoreError> {
    let rows = client
        .query(
            "select \
                seq, actor_id, document_id, checkpoint_anchor_seq, history_kind, base_text, body_text, conflicted, \
                (extract(epoch from created_at) * 1000)::bigint as timestamp_ms \
             from document_body_revisions \
             where workspace_id = $1 and document_id = $2 \
             order by seq desc \
             limit $3",
            &[&workspace_id, &document_id, &(limit as i64)],
        )
        .await?;
    let mut revisions = rows
        .into_iter()
        .map(|row| {
            let kind =
                RetainedBodyHistoryKind::parse(row.get::<_, String>("history_kind").as_str())
                    .map_err(StoreError::new)?;
            Ok(FileBodyRevision {
                seq: row.get::<_, i64>("seq") as u64,
                workspace_cursor: 0,
                actor_id: row.get::<_, String>("actor_id"),
                document_id: row.get::<_, String>("document_id"),
                checkpoint_anchor_seq: row
                    .get::<_, Option<i64>>("checkpoint_anchor_seq")
                    .map(|seq| seq as u64),
                history_kind: kind,
                base_text: row.get::<_, String>("base_text"),
                body_text: row.get::<_, String>("body_text"),
                conflicted: row.get::<_, bool>("conflicted"),
                timestamp_ms: row.get::<_, i64>("timestamp_ms") as u128,
            }
            .to_public_revision())
        })
        .collect::<Result<Vec<_>, StoreError>>()?;
    revisions.reverse();
    Ok(revisions)
}

pub(crate) async fn postgres_preview_redact_document_body_history(
    client: &Client,
    request: &PreviewRedactDocumentBodyHistoryRequest,
) -> Result<Vec<DocumentBodyRedactionMatch>, StoreError> {
    let rows = client
        .query(
            "select seq, actor_id, document_id, checkpoint_anchor_seq, history_kind, base_text, body_text, conflicted, \
                (extract(epoch from created_at) * 1000)::bigint as timestamp_ms \
             from document_body_revisions \
             where workspace_id = $1 and document_id = $2 \
             order by seq asc",
            &[&request.workspace_id, &request.document_id],
        )
        .await?;
    let revisions = rows
        .into_iter()
        .map(|row| {
            let kind =
                RetainedBodyHistoryKind::parse(row.get::<_, String>("history_kind").as_str())
                    .map_err(StoreError::new)?;
            Ok(FileBodyRevision {
                seq: row.get::<_, i64>("seq") as u64,
                workspace_cursor: 0,
                actor_id: row.get::<_, String>("actor_id"),
                document_id: row.get::<_, String>("document_id"),
                checkpoint_anchor_seq: row
                    .get::<_, Option<i64>>("checkpoint_anchor_seq")
                    .map(|seq| seq as u64),
                history_kind: kind,
                base_text: row.get::<_, String>("base_text"),
                body_text: row.get::<_, String>("body_text"),
                conflicted: row.get::<_, bool>("conflicted"),
                timestamp_ms: row.get::<_, i64>("timestamp_ms") as u128,
            })
        })
        .collect::<Result<Vec<_>, StoreError>>()?;
    let matches = retained_redaction_matches(
        revisions,
        &request.document_id,
        &request.exact_text,
        request.limit,
    )?;
    if matches.is_empty() {
        return Err(StoreError::new(format!(
            "document {} has no retained body history matching {:?} in workspace {}",
            request.document_id, request.exact_text, request.workspace_id
        )));
    }
    Ok(matches)
}

pub(crate) async fn postgres_preview_purge_document_body_history(
    client: &Client,
    request: &PreviewPurgeDocumentBodyHistoryRequest,
) -> Result<Vec<DocumentBodyPurgeMatch>, StoreError> {
    let rows = client
        .query(
            "select seq, actor_id, document_id, checkpoint_anchor_seq, history_kind, base_text, body_text, conflicted, \
                (extract(epoch from created_at) * 1000)::bigint as timestamp_ms \
             from document_body_revisions \
             where workspace_id = $1 and document_id = $2 \
             order by seq asc",
            &[&request.workspace_id, &request.document_id],
        )
        .await?;
    let revisions = rows
        .into_iter()
        .map(|row| {
            let kind =
                RetainedBodyHistoryKind::parse(row.get::<_, String>("history_kind").as_str())
                    .map_err(StoreError::new)?;
            Ok(FileBodyRevision {
                seq: row.get::<_, i64>("seq") as u64,
                workspace_cursor: 0,
                actor_id: row.get::<_, String>("actor_id"),
                document_id: row.get::<_, String>("document_id"),
                checkpoint_anchor_seq: row
                    .get::<_, Option<i64>>("checkpoint_anchor_seq")
                    .map(|seq| seq as u64),
                history_kind: kind,
                base_text: row.get::<_, String>("base_text"),
                body_text: row.get::<_, String>("body_text"),
                conflicted: row.get::<_, bool>("conflicted"),
                timestamp_ms: row.get::<_, i64>("timestamp_ms") as u128,
            })
        })
        .collect::<Result<Vec<_>, StoreError>>()?;
    let matches = retained_purge_matches(revisions, &request.document_id, request.limit);
    if matches.is_empty() {
        return Err(StoreError::new(format!(
            "document {} has no retained body history in workspace {}",
            request.document_id, request.workspace_id
        )));
    }
    Ok(matches)
}

pub(crate) async fn postgres_enforce_history_compaction_policy(
    transaction: &tokio_postgres::Transaction<'_>,
    workspace_id: &str,
    document_id: &str,
) -> Result<(), StoreError> {
    let Some(path_row) = transaction
        .query_opt(
            "select mount_path, relative_path from document_paths \
             where workspace_id = $1 and document_id = $2 and deleted = false",
            &[&workspace_id, &document_id],
        )
        .await?
    else {
        return Ok(());
    };
    let repo_relative_path = PathBuf::from(path_row.get::<_, String>("mount_path"))
        .join(path_row.get::<_, String>("relative_path"));
    let resolved = resolve_history_compaction_policy(
        &postgres_read_history_compaction_policies(transaction, workspace_id).await?,
        &repo_relative_path,
    );
    let rows = transaction
        .query(
            "select seq, actor_id, document_id, checkpoint_anchor_seq, history_kind, base_text, body_text, conflicted, \
                (extract(epoch from created_at) * 1000)::bigint as timestamp_ms \
             from document_body_revisions \
             where workspace_id = $1 and document_id = $2 \
             order by seq asc",
            &[&workspace_id, &document_id],
        )
        .await?;
    let original = rows
        .into_iter()
        .map(|row| {
            let kind =
                RetainedBodyHistoryKind::parse(row.get::<_, String>("history_kind").as_str())
                    .map_err(StoreError::new)?;
            Ok(FileBodyRevision {
                seq: row.get::<_, i64>("seq") as u64,
                workspace_cursor: 0,
                actor_id: row.get::<_, String>("actor_id"),
                document_id: row.get::<_, String>("document_id"),
                checkpoint_anchor_seq: row
                    .get::<_, Option<i64>>("checkpoint_anchor_seq")
                    .map(|seq| seq as u64),
                history_kind: kind,
                base_text: row.get::<_, String>("base_text"),
                body_text: row.get::<_, String>("body_text"),
                conflicted: row.get::<_, bool>("conflicted"),
                timestamp_ms: row.get::<_, i64>("timestamp_ms") as u128,
            })
        })
        .collect::<Result<Vec<_>, StoreError>>()?;
    let compacted = compact_document_body_revisions(&original, document_id, &resolved.policy)?;
    if compacted == original {
        return Ok(());
    }
    let compacted_by_seq = compacted
        .iter()
        .map(|revision| (revision.seq, revision))
        .collect::<HashMap<_, _>>();
    for revision in &compacted {
        transaction
            .execute(
                "update document_body_revisions \
                 set checkpoint_anchor_seq = $3, history_kind = $4, base_text = $5, body_text = $6, conflicted = $7 \
                 where workspace_id = $1 and seq = $2",
                &[
                    &workspace_id,
                    &(revision.seq as i64),
                    &revision.checkpoint_anchor_seq.map(|seq| seq as i64),
                    &revision.history_kind.as_str(),
                    &revision.base_text,
                    &revision.body_text,
                    &revision.conflicted,
                ],
            )
            .await?;
    }
    let dropped = original
        .iter()
        .filter(|revision| !compacted_by_seq.contains_key(&revision.seq))
        .map(|revision| revision.seq as i64)
        .collect::<Vec<_>>();
    if !dropped.is_empty() {
        transaction
            .execute(
                "delete from document_body_revisions \
                 where workspace_id = $1 and document_id = $2 and seq = any($3)",
                &[&workspace_id, &document_id, &dropped],
            )
            .await?;
    }
    Ok(())
}

pub(crate) async fn postgres_purge_document_body_history(
    transaction: &tokio_postgres::Transaction<'_>,
    request: &PurgeDocumentBodyHistoryRequest,
) -> Result<(), StoreError> {
    let matched_rows = transaction
        .query(
            "select seq from document_body_revisions \
             where workspace_id = $1 and document_id = $2 and (base_text <> '' or body_text <> '') \
             order by seq asc",
            &[&request.workspace_id, &request.document_id],
        )
        .await?;
    let matched_seqs = matched_rows
        .into_iter()
        .map(|row| row.get::<_, i64>(0) as u64)
        .collect::<Vec<_>>();
    if matched_seqs.is_empty() {
        return Err(StoreError::new(format!(
            "document {} has no retained body history in workspace {}",
            request.document_id, request.workspace_id
        )));
    }
    ensure_expected_history_match_set(
        &request.document_id,
        request.expected_match_seqs.as_ref(),
        &matched_seqs,
        "purge",
    )?;
    let matched = transaction
        .execute(
            "update document_body_revisions \
             set base_text = '', body_text = '' \
             where workspace_id = $1 and document_id = $2 and (base_text <> '' or body_text <> '')",
            &[&request.workspace_id, &request.document_id],
        )
        .await?;
    if matched == 0 {
        return Err(StoreError::new(format!(
            "document {} has no retained body history in workspace {}",
            request.document_id, request.workspace_id
        )));
    }
    transaction
        .execute(
            "delete from document_body_updates where workspace_id = $1 and document_id = $2",
            &[&request.workspace_id, &request.document_id],
        )
        .await?;
    let live_path = transaction
        .query_opt(
            "select mount_path, relative_path from document_paths \
             where workspace_id = $1 and document_id = $2 and deleted = false",
            &[&request.workspace_id, &request.document_id],
        )
        .await?;
    let mount_relative_path = live_path
        .as_ref()
        .map(|row| row.get::<_, String>("mount_path"));
    let relative_path = live_path
        .as_ref()
        .map(|row| row.get::<_, String>("relative_path"));
    insert_event_tx(
        transaction,
        &request.workspace_id,
        &request.actor_id,
        Some(&request.document_id),
        mount_relative_path.as_deref(),
        relative_path.as_deref(),
        ProvenanceEventKind::DocumentHistoryPurged,
        &purge_history_summary(
            request.document_id.as_str(),
            mount_relative_path.as_deref(),
            relative_path.as_deref(),
        ),
    )
    .await?;
    Ok(())
}

pub(crate) async fn postgres_redact_document_body_history(
    transaction: &tokio_postgres::Transaction<'_>,
    request: &RedactDocumentBodyHistoryRequest,
) -> Result<(), StoreError> {
    let rows = transaction
        .query(
            "select seq, actor_id, document_id, checkpoint_anchor_seq, history_kind, base_text, body_text, conflicted, \
                (extract(epoch from created_at) * 1000)::bigint as timestamp_ms \
             from document_body_revisions \
             where workspace_id = $1 and document_id = $2 \
             order by seq asc",
            &[&request.workspace_id, &request.document_id],
        )
        .await?;
    let revisions = rows
        .into_iter()
        .map(|row| {
            let kind =
                RetainedBodyHistoryKind::parse(row.get::<_, String>("history_kind").as_str())
                    .map_err(StoreError::new)?;
            Ok(FileBodyRevision {
                seq: row.get::<_, i64>("seq") as u64,
                workspace_cursor: 0,
                actor_id: row.get::<_, String>("actor_id"),
                document_id: row.get::<_, String>("document_id"),
                checkpoint_anchor_seq: row
                    .get::<_, Option<i64>>("checkpoint_anchor_seq")
                    .map(|seq| seq as u64),
                history_kind: kind,
                base_text: row.get::<_, String>("base_text"),
                body_text: row.get::<_, String>("body_text"),
                conflicted: row.get::<_, bool>("conflicted"),
                timestamp_ms: row.get::<_, i64>("timestamp_ms") as u128,
            })
        })
        .collect::<Result<Vec<_>, StoreError>>()?;

    let mut matched_seqs = Vec::new();
    for revision in revisions {
        let Some(redacted) = revision.redacted(&request.exact_text)? else {
            continue;
        };
        matched_seqs.push(redacted.seq);
        transaction
            .execute(
                "update document_body_revisions \
                 set history_kind = $3, base_text = $4, body_text = $5, conflicted = $6 \
                 where workspace_id = $1 and seq = $2",
                &[
                    &request.workspace_id,
                    &(redacted.seq as i64),
                    &redacted.history_kind.as_str(),
                    &redacted.base_text,
                    &redacted.body_text,
                    &redacted.conflicted,
                ],
            )
            .await?;
    }
    if matched_seqs.is_empty() {
        return Err(StoreError::new(format!(
            "document {} has no retained body history matching {:?} in workspace {}",
            request.document_id, request.exact_text, request.workspace_id
        )));
    }
    ensure_expected_history_match_set(
        &request.document_id,
        request.expected_match_seqs.as_ref(),
        &matched_seqs,
        "redaction",
    )?;
    let live_path = transaction
        .query_opt(
            "select mount_path, relative_path from document_paths \
             where workspace_id = $1 and document_id = $2 and deleted = false",
            &[&request.workspace_id, &request.document_id],
        )
        .await?;
    let mount_relative_path = live_path
        .as_ref()
        .map(|row| row.get::<_, String>("mount_path"));
    let relative_path = live_path
        .as_ref()
        .map(|row| row.get::<_, String>("relative_path"));
    insert_event_tx(
        transaction,
        &request.workspace_id,
        &request.actor_id,
        Some(&request.document_id),
        mount_relative_path.as_deref(),
        relative_path.as_deref(),
        ProvenanceEventKind::DocumentHistoryRedacted,
        &redact_history_summary(
            request.document_id.as_str(),
            mount_relative_path.as_deref(),
            relative_path.as_deref(),
        ),
    )
    .await?;
    Ok(())
}

pub(crate) async fn postgres_list_path_revisions(
    client: &Client,
    workspace_id: &str,
    document_id: &str,
    limit: usize,
) -> Result<Vec<DocumentPathRevision>, StoreError> {
    let rows = client
        .query(
            "select \
                seq, actor_id, document_id, mount_path, relative_path, deleted, event_kind, \
                (extract(epoch from created_at) * 1000)::bigint as timestamp_ms \
             from document_path_history \
             where workspace_id = $1 and document_id = $2 \
             order by seq desc \
             limit $3",
            &[&workspace_id, &document_id, &(limit as i64)],
        )
        .await?;
    let mut revisions = rows
        .into_iter()
        .map(|row| DocumentPathRevision {
            seq: row.get::<_, i64>("seq") as u64,
            actor_id: row.get::<_, String>("actor_id"),
            document_id: row.get::<_, String>("document_id"),
            mount_path: row.get::<_, String>("mount_path"),
            relative_path: row.get::<_, String>("relative_path"),
            deleted: row.get::<_, bool>("deleted"),
            event_kind: row.get::<_, String>("event_kind"),
            timestamp_ms: row.get::<_, i64>("timestamp_ms") as u128,
        })
        .collect::<Vec<_>>();
    revisions.reverse();
    Ok(revisions)
}

pub(crate) async fn postgres_resolve_document_by_historical_path(
    client: &Client,
    workspace_id: &str,
    mount_path: &str,
    relative_path: &str,
) -> Result<DocumentId, StoreError> {
    let row = client
        .query_opt(
            "select document_id from document_path_history \
             where workspace_id = $1 and mount_path = $2 and relative_path = $3 \
             order by seq desc \
             limit 1",
            &[&workspace_id, &mount_path, &relative_path],
        )
        .await?;
    row.map(|row| DocumentId::new(row.get::<_, String>("document_id")))
        .ok_or_else(|| {
            StoreError::new(format!(
                "no document path history found at {mount_path}/{relative_path}"
            ))
        })
}

pub(crate) async fn postgres_reconstruct_workspace_at_cursor(
    client: &Client,
    workspace_id: &str,
    cursor: u64,
) -> Result<BootstrapSnapshot, StoreError> {
    let path_rows = client
        .query(
            "select distinct on (document_id) \
                document_id, mount_path, relative_path, deleted \
             from document_path_history \
             where workspace_id = $1 and workspace_cursor <= $2 \
             order by document_id, workspace_cursor desc, seq desc",
            &[&workspace_id, &(cursor as i64)],
        )
        .await?;
    let body_rows = client
        .query(
            "select \
                seq, workspace_cursor, document_id, checkpoint_anchor_seq, history_kind, base_text, body_text, conflicted \
             from document_body_revisions \
             where workspace_id = $1 and workspace_cursor <= $2 \
             order by workspace_cursor asc, seq asc",
            &[&workspace_id, &(cursor as i64)],
        )
        .await?;

    let body_map = replay_body_revision_run(body_rows.into_iter().map(|row| {
        let kind = RetainedBodyHistoryKind::parse(row.get::<_, String>("history_kind").as_str())
            .expect("stored retained body history kind should parse");
        FileBodyRevision {
            seq: row.get::<_, i64>("seq") as u64,
            workspace_cursor: row.get::<_, i64>("workspace_cursor") as u64,
            actor_id: String::new(),
            document_id: row.get::<_, String>("document_id"),
            checkpoint_anchor_seq: row
                .get::<_, Option<i64>>("checkpoint_anchor_seq")
                .map(|seq| seq as u64),
            history_kind: kind,
            base_text: row.get::<_, String>("base_text"),
            body_text: row.get::<_, String>("body_text"),
            conflicted: row.get::<_, bool>("conflicted"),
            timestamp_ms: 0,
        }
    }));

    let mut entries = path_rows
        .into_iter()
        .map(|row| ManifestEntry {
            document_id: DocumentId::new(row.get::<_, String>("document_id")),
            mount_relative_path: row.get::<_, String>("mount_path").into(),
            relative_path: row.get::<_, String>("relative_path").into(),
            kind: DocumentKind::Text,
            deleted: row.get::<_, bool>("deleted"),
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        left.mount_relative_path
            .cmp(&right.mount_relative_path)
            .then_with(|| left.relative_path.cmp(&right.relative_path))
            .then_with(|| left.document_id.as_str().cmp(right.document_id.as_str()))
    });

    Ok(snapshot_from_manifest_entries(entries, |document_id| {
        body_map.get(document_id.as_str()).cloned()
    }))
}

pub(crate) async fn postgres_restore_workspace_at_cursor(
    transaction: &tokio_postgres::Transaction<'_>,
    request: &RestoreWorkspaceRequest,
) -> Result<(), StoreError> {
    let current_cursor = current_workspace_cursor_tx(transaction, &request.workspace_id).await?;
    let provided_cursor = request
        .based_on_cursor
        .ok_or_else(|| StoreError::new("workspace restore missing based_on_cursor precondition"))?;
    if provided_cursor != current_cursor {
        return Err(StoreError::conflict(
            "stale_cursor",
            format!(
                "workspace restore based on stale cursor {provided_cursor}; current workspace cursor is {current_cursor}"
            ),
        ));
    }
    if request.cursor > current_cursor {
        return Err(StoreError::new(format!(
            "workspace restore target cursor {} is newer than current workspace cursor {}",
            request.cursor, current_cursor
        )));
    }

    let current_snapshot =
        postgres_current_workspace_snapshot(transaction, &request.workspace_id).await?;
    let target_snapshot = postgres_reconstruct_workspace_at_cursor(
        transaction.client(),
        &request.workspace_id,
        request.cursor,
    )
    .await?;
    let restored_snapshot =
        build_restored_live_workspace_snapshot(&current_snapshot, &target_snapshot);
    let changes =
        diff_workspace_restore_changes(&current_snapshot, &restored_snapshot, request.cursor);

    transaction
        .execute(
            "update document_paths set deleted = true, updated_at = now() where workspace_id = $1",
            &[&request.workspace_id],
        )
        .await?;

    for entry in &restored_snapshot.manifest.entries {
        let existing = transaction
            .query_opt(
                "select 1 from document_paths where workspace_id = $1 and document_id = $2",
                &[&request.workspace_id, &entry.document_id.as_str()],
            )
            .await?;
        if existing.is_some() {
            transaction
                .execute(
                    "update document_paths set mount_path = $3, relative_path = $4, deleted = $5, updated_at = now() \
                     where workspace_id = $1 and document_id = $2",
                    &[
                        &request.workspace_id,
                        &entry.document_id.as_str(),
                        &entry.mount_relative_path.display().to_string(),
                        &entry.relative_path.display().to_string(),
                        &entry.deleted,
                    ],
                )
                .await?;
        } else {
            transaction
                .execute(
                    "insert into document_paths \
                     (document_id, workspace_id, mount_path, relative_path, deleted, manifest_version) \
                     values ($1, $2, $3, $4, $5, 1)",
                    &[
                        &entry.document_id.as_str(),
                        &request.workspace_id,
                        &entry.mount_relative_path.display().to_string(),
                        &entry.relative_path.display().to_string(),
                        &entry.deleted,
                    ],
                )
                .await?;
        }
    }

    let body_persistence = PostgresBodyPersistence::new(transaction, &request.workspace_id);
    for body in &restored_snapshot.bodies {
        let state = FULL_TEXT_BODY_MODEL.state_from_materialized_text(body.text.clone());
        body_persistence
            .write_current_state(body.document_id.as_str(), &state)
            .await?;
    }

    for change in changes {
        let event_cursor = insert_event_tx(
            transaction,
            &request.workspace_id,
            &request.actor_id,
            Some(change.document_id.as_str()),
            Some(&change.path.mount_path),
            Some(&change.path.relative_path),
            change.kind.clone(),
            &change.summary,
        )
        .await?;
        if let Some(body) = change.body {
            body_persistence
                .append_retained_history(
                    event_cursor,
                    &request.actor_id,
                    change.document_id.as_str(),
                    &FULL_TEXT_BODY_MODEL.checkpoint_history(body.base_text, body.body_text),
                )
                .await?;
        }
        insert_path_revision_tx(
            transaction,
            &request.workspace_id,
            change.document_id.as_str(),
            event_cursor,
            &request.actor_id,
            &change.path.mount_path,
            &change.path.relative_path,
            change.path.deleted,
            &change.path.event_kind,
        )
        .await?;
    }

    Ok(())
}

async fn postgres_current_workspace_snapshot(
    transaction: &tokio_postgres::Transaction<'_>,
    workspace_id: &str,
) -> Result<BootstrapSnapshot, StoreError> {
    let rows = transaction
        .query(
            "select \
                dp.document_id, \
                dp.mount_path, \
                dp.relative_path, \
                d.kind, \
                dp.deleted, \
                coalesce(dbs.state_kind, 'full_text_merge_v1') as state_kind, \
                coalesce(dbs.body_text, '') as body_text \
             from document_paths dp \
             join documents d on d.id = dp.document_id \
             left join document_body_snapshots dbs on dbs.document_id = dp.document_id \
             where dp.workspace_id = $1 \
             order by dp.mount_path, dp.relative_path",
            &[&workspace_id],
        )
        .await?;
    snapshot_from_current_rows(rows, |_| Ok(DocumentKind::Text))
}

fn current_time_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time before unix epoch")
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::body_state::{BodyConvergenceEngine, YrsConvergenceBodyEngine};

    fn append_and_compact_revision(
        revisions: &mut Vec<FileBodyRevision>,
        seq: u64,
        document_id: &str,
        previous_text: &str,
        next_text: &str,
    ) {
        let payload = if seq == 1 {
            FULL_TEXT_BODY_MODEL.checkpoint_history("", next_text)
        } else {
            YrsConvergenceBodyEngine
                .apply_update(
                    "tester",
                    previous_text,
                    &FULL_TEXT_BODY_MODEL.state_from_materialized_text(previous_text),
                    next_text,
                )
                .retained_history()
                .clone()
        };
        let checkpoint_anchor_seq = if payload.kind() == RetainedBodyHistoryKind::YrsTextUpdateV1 {
            super::history_compaction::latest_checkpoint_anchor_seq(revisions.clone(), document_id)
        } else {
            Some(seq)
        };
        revisions.push(FileBodyRevision::from_retained_history(
            seq,
            seq,
            "tester".to_owned(),
            document_id.to_owned(),
            checkpoint_anchor_seq,
            &payload,
            seq as u128,
        ));
        *revisions = compact_document_body_revisions(
            revisions,
            document_id,
            &HistoryCompactionPolicy {
                revisions: 2,
                frequency: 2,
            },
        )
        .expect("compaction should succeed");
    }

    #[test]
    fn repeated_compaction_preserves_latest_text_across_checkpoint_update_runs() {
        let document_id = "doc-1";
        let mut revisions = Vec::new();
        let mut previous_text = String::new();
        for (seq, next_text) in [
            "<p>revision one</p>\n",
            "<p>revision two</p>\n",
            "<p>revision three</p>\n",
            "<p>revision four</p>\n",
            "<p>revision five</p>\n",
            "<p>revision six</p>\n",
        ]
        .into_iter()
        .enumerate()
        {
            let seq = seq as u64 + 1;
            append_and_compact_revision(
                &mut revisions,
                seq,
                document_id,
                &previous_text,
                next_text,
            );
            let reconstructed = replay_body_revision_run(revisions.clone());
            assert_eq!(
                reconstructed
                    .get(document_id)
                    .expect("document state should exist")
                    .materialized_text(),
                next_text,
                "replay after seq {seq} should preserve latest text; retained revisions: {revisions:#?}",
            );
            previous_text = next_text.to_owned();
        }
    }
}
