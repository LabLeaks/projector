/**
@module PROJECTOR.SERVER.FILE_HISTORY
Owns file-backed retained-history storage, preview, compaction-policy persistence, and workspace-history reconstruction and restore.
*/
// @fileimplements PROJECTOR.SERVER.FILE_HISTORY
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use projector_domain::{
    BootstrapSnapshot, ClearHistoryCompactionPolicyRequest, DocumentBodyPurgeMatch,
    DocumentBodyRedactionMatch, DocumentBodyRevision, DocumentId, DocumentKind,
    DocumentPathRevision, GetHistoryCompactionPolicyResponse, ManifestEntry,
    PreviewPurgeDocumentBodyHistoryRequest, PreviewRedactDocumentBodyHistoryRequest,
    ProvenanceEvent, ProvenanceEventKind, PurgeDocumentBodyHistoryRequest,
    RedactDocumentBodyHistoryRequest, RestoreWorkspaceRequest, SetHistoryCompactionPolicyRequest,
};

use super::StoreError;
use super::bodies::{file_persist_workspace_snapshot, file_read_workspace_snapshot};
use super::body_persistence::{FileBodyPersistence, SnapshotBodyPersistence};
use super::body_projection::snapshot_from_manifest_entries;
use super::body_state::{BodyStateModel, FULL_TEXT_BODY_MODEL};
use super::history::{
    FileBodyRevision, FilePathRevision, current_time_ms, effective_workspace_cursor,
};
use super::history_compaction::{
    StoredHistoryCompactionPolicyOverride, compact_document_body_revisions,
    history_compaction_response, replay_body_revision_run, resolve_history_compaction_policy,
};
use super::history_restore::{
    build_restored_live_workspace_snapshot, diff_workspace_restore_changes,
};
use super::history_surgery::{
    ensure_expected_history_match_set, purge_history_summary, redact_history_summary,
    retained_purge_matches, retained_redaction_matches,
};
use super::provenance::{file_append_workspace_event, file_workspace_cursor};
use super::workspaces::workspace_dir;

fn file_body_revisions_path(state_dir: &Path, workspace_id: &str) -> std::path::PathBuf {
    workspace_dir(state_dir, workspace_id).join("body_revisions.json")
}

fn file_history_compaction_policies_path(
    state_dir: &Path,
    workspace_id: &str,
) -> std::path::PathBuf {
    workspace_dir(state_dir, workspace_id).join("history_compaction_policies.json")
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
