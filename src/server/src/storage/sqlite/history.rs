/**
@module PROJECTOR.SERVER.SQLITE_HISTORY
Owns SQLite event and revision reads for list, discovery, and historical path resolution over append-only server history rows.
*/
// @fileimplements PROJECTOR.SERVER.SQLITE_HISTORY
use projector_domain::{
    ClearHistoryCompactionPolicyRequest, DocumentBodyPurgeMatch, DocumentBodyRedactionMatch,
    DocumentBodyRevision, DocumentId, DocumentPathRevision, GetHistoryCompactionPolicyResponse,
    HistoryCompactionPolicy, PreviewPurgeDocumentBodyHistoryRequest,
    PreviewRedactDocumentBodyHistoryRequest, ProvenanceEvent, ProvenanceEventKind,
    PurgeDocumentBodyHistoryRequest, RedactDocumentBodyHistoryRequest, SetHistoryCompactionPolicyRequest,
};
use rusqlite::{Connection, OptionalExtension, params};

use super::super::StoreError;
pub(super) use super::super::history::{FileBodyRevision, FilePathRevision};
use super::state::{
    append_event, decode_json, effective_workspace_cursor, encode_json,
    load_required_workspace_state, make_event,
};

fn read_history_compaction_policies(
    connection: &Connection,
    workspace_id: &str,
) -> Result<Vec<super::super::history::StoredHistoryCompactionPolicyOverride>, StoreError> {
    let mut stmt = connection.prepare(
        "select repo_relative_path, revisions, frequency \
         from history_compaction_policies \
         where workspace_id = ?1 \
         order by repo_relative_path asc",
    )?;
    let rows = stmt.query_map(params![workspace_id], |row| {
        Ok(super::super::history::StoredHistoryCompactionPolicyOverride {
            repo_relative_path: std::path::PathBuf::from(row.get::<_, String>(0)?),
            policy: HistoryCompactionPolicy {
                revisions: row.get::<_, i64>(1)? as usize,
                frequency: row.get::<_, i64>(2)? as usize,
            },
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(StoreError::from)
}

pub(super) fn get_history_compaction_policy(
    connection: &Connection,
    workspace_id: &str,
    repo_relative_path: &str,
) -> Result<GetHistoryCompactionPolicyResponse, StoreError> {
    Ok(super::super::history::history_compaction_response(
        &read_history_compaction_policies(connection, workspace_id)?,
        std::path::Path::new(repo_relative_path),
    ))
}

pub(super) fn set_history_compaction_policy(
    connection: &rusqlite::Transaction<'_>,
    request: &SetHistoryCompactionPolicyRequest,
) -> Result<(), StoreError> {
    connection.execute(
        "insert into history_compaction_policies (workspace_id, repo_relative_path, revisions, frequency) \
         values (?1, ?2, ?3, ?4) \
         on conflict(workspace_id, repo_relative_path) do update set \
           revisions = excluded.revisions, \
           frequency = excluded.frequency",
        params![
            request.workspace_id,
            request.repo_relative_path,
            request.policy.revisions as i64,
            request.policy.frequency as i64,
        ],
    )?;
    Ok(())
}

pub(super) fn clear_history_compaction_policy(
    connection: &rusqlite::Transaction<'_>,
    request: &ClearHistoryCompactionPolicyRequest,
) -> Result<bool, StoreError> {
    Ok(connection.execute(
        "delete from history_compaction_policies where workspace_id = ?1 and repo_relative_path = ?2",
        params![request.workspace_id, request.repo_relative_path],
    )? > 0)
}

pub(super) fn read_events_since(
    connection: &Connection,
    workspace_id: &str,
    since_cursor: u64,
) -> Result<Vec<ProvenanceEvent>, StoreError> {
    let mut stmt = connection.prepare(
        "select event_json from events where workspace_id = ?1 and seq > ?2 order by seq asc",
    )?;
    let rows = stmt.query_map(params![workspace_id, since_cursor as i64], |row| {
        row.get::<_, String>(0)
    })?;
    rows.map(|row| decode_json(&row?)).collect()
}

pub(super) fn read_recent_events(
    connection: &Connection,
    workspace_id: &str,
    limit: usize,
) -> Result<Vec<ProvenanceEvent>, StoreError> {
    let mut stmt = connection.prepare(
        "select event_json from events where workspace_id = ?1 order by seq desc limit ?2",
    )?;
    let rows = stmt.query_map(params![workspace_id, limit as i64], |row| {
        row.get::<_, String>(0)
    })?;
    let mut events = rows
        .map(|row| decode_json::<ProvenanceEvent>(&row?))
        .collect::<Result<Vec<_>, _>>()?;
    events.reverse();
    Ok(events)
}

pub(super) fn read_last_event_timestamp(
    connection: &Connection,
    workspace_id: &str,
) -> Result<Option<u128>, StoreError> {
    connection
        .query_row(
            "select event_json from events where workspace_id = ?1 order by seq desc limit 1",
            params![workspace_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .map(|encoded| Ok(decode_json::<ProvenanceEvent>(&encoded)?.timestamp_ms))
        .transpose()
}

pub(crate) fn read_body_revisions(
    connection: &Connection,
    workspace_id: &str,
) -> Result<Vec<FileBodyRevision>, StoreError> {
    let mut stmt = connection.prepare(
        "select revision_json from body_revisions where workspace_id = ?1 order by seq asc",
    )?;
    let rows = stmt.query_map(params![workspace_id], |row| row.get::<_, String>(0))?;
    rows.map(|row| decode_json(&row?)).collect()
}

pub(super) fn read_path_history(
    connection: &Connection,
    workspace_id: &str,
) -> Result<Vec<FilePathRevision>, StoreError> {
    let mut stmt = connection.prepare(
        "select revision_json from path_revisions where workspace_id = ?1 order by seq asc",
    )?;
    let rows = stmt.query_map(params![workspace_id], |row| row.get::<_, String>(0))?;
    rows.map(|row| decode_json(&row?)).collect()
}

pub(super) fn list_body_revisions(
    connection: &Connection,
    workspace_id: &str,
    document_id: &str,
    limit: usize,
) -> Result<Vec<DocumentBodyRevision>, StoreError> {
    let mut revisions = read_body_revisions(connection, workspace_id)?
        .into_iter()
        .filter(|revision| revision.document_id == document_id)
        .map(|revision| revision.to_public_revision())
        .collect::<Vec<_>>();
    if revisions.len() > limit {
        revisions = revisions.split_off(revisions.len() - limit);
    }
    Ok(revisions)
}

pub(super) fn preview_redact_document_body_history(
    connection: &Connection,
    request: &PreviewRedactDocumentBodyHistoryRequest,
) -> Result<Vec<DocumentBodyRedactionMatch>, StoreError> {
    let matches = super::super::history::retained_redaction_matches(
        read_body_revisions(connection, &request.workspace_id)?,
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

pub(super) fn preview_purge_document_body_history(
    connection: &Connection,
    request: &PreviewPurgeDocumentBodyHistoryRequest,
) -> Result<Vec<DocumentBodyPurgeMatch>, StoreError> {
    let matches = super::super::history::retained_purge_matches(
        read_body_revisions(connection, &request.workspace_id)?,
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

pub(crate) fn enforce_history_compaction_policy(
    connection: &rusqlite::Transaction<'_>,
    workspace_id: &str,
    document_id: &str,
) -> Result<(), StoreError> {
    let state = load_required_workspace_state(connection, workspace_id)?;
    let Some(entry) = state
        .snapshot
        .manifest
        .entries
        .iter()
        .find(|entry| !entry.deleted && entry.document_id.as_str() == document_id)
    else {
        return Ok(());
    };
    let repo_relative_path = entry.mount_relative_path.join(&entry.relative_path);
    let resolved = super::super::history::resolve_history_compaction_policy(
        &read_history_compaction_policies(connection, workspace_id)?,
        &repo_relative_path,
    );
    let original = read_body_revisions(connection, workspace_id)?
        .into_iter()
        .filter(|revision| revision.document_id == document_id)
        .collect::<Vec<_>>();
    let compacted = super::super::history::compact_document_body_revisions(
        &original,
        document_id,
        &resolved.policy,
    )?;
    if compacted == original {
        return Ok(());
    }
    let compacted_by_seq = compacted
        .iter()
        .map(|revision| (revision.seq, revision))
        .collect::<std::collections::HashMap<_, _>>();
    for revision in &compacted {
        connection.execute(
            "update body_revisions set revision_json = ?3 where workspace_id = ?1 and seq = ?2",
            params![workspace_id, revision.seq as i64, encode_json(revision)?],
        )?;
    }
    for revision in original
        .iter()
        .filter(|revision| !compacted_by_seq.contains_key(&revision.seq))
    {
        connection.execute(
            "delete from body_revisions where workspace_id = ?1 and seq = ?2",
            params![workspace_id, revision.seq as i64],
        )?;
    }
    Ok(())
}

pub(super) fn list_path_revisions(
    connection: &Connection,
    workspace_id: &str,
    document_id: &str,
    limit: usize,
) -> Result<Vec<DocumentPathRevision>, StoreError> {
    let mut revisions = read_path_history(connection, workspace_id)?
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

pub(super) fn resolve_document_by_historical_path(
    connection: &Connection,
    workspace_id: &str,
    mount_path: &str,
    relative_path: &str,
) -> Result<DocumentId, StoreError> {
    read_path_history(connection, workspace_id)?
        .into_iter()
        .filter(|revision| {
            revision.mount_path == mount_path && revision.relative_path == relative_path
        })
        .max_by_key(|revision| {
            (
                effective_workspace_cursor(revision.seq, revision.workspace_cursor),
                revision.seq,
            )
        })
        .map(|revision| DocumentId::new(revision.document_id))
        .ok_or_else(|| {
            StoreError::new(format!(
                "no document path history found at {mount_path}/{relative_path}"
            ))
        })
}

pub(super) fn purge_document_body_history(
    connection: &rusqlite::Transaction<'_>,
    request: &PurgeDocumentBodyHistoryRequest,
) -> Result<(), StoreError> {
    let matched_seqs = read_body_revisions(connection, &request.workspace_id)?
        .into_iter()
        .filter(|revision| {
            revision.document_id == request.document_id
                && (!revision.base_text.is_empty() || !revision.body_text.is_empty())
        })
        .map(|revision| revision.seq)
        .collect::<Vec<_>>();
    if matched_seqs.is_empty() {
        return Err(StoreError::new(format!(
            "document {} has no retained body history in workspace {}",
            request.document_id, request.workspace_id
        )));
    }
    super::super::history::ensure_expected_history_match_set(
        &request.document_id,
        request.expected_match_seqs.as_ref(),
        &matched_seqs,
        "purge",
    )?;

    let matched = connection.execute(
        "update body_revisions \
         set revision_json = json_set(revision_json, '$.base_text', '', '$.body_text', '') \
         where workspace_id = ?1 and json_extract(revision_json, '$.document_id') = ?2 \
           and (json_extract(revision_json, '$.base_text') <> '' or json_extract(revision_json, '$.body_text') <> '')",
        params![request.workspace_id, request.document_id],
    )?;
    debug_assert!(matched > 0);

    let live_path = connection
        .query_row(
            "select mount_path, relative_path from document_paths \
             where workspace_id = ?1 and document_id = ?2 and deleted = false",
            params![request.workspace_id, request.document_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    let mut state = load_required_workspace_state(connection, &request.workspace_id)?;
    let mount_relative_path = live_path.as_ref().map(|(mount, _)| mount.clone());
    let relative_path = live_path.as_ref().map(|(_, relative)| relative.clone());
    let event = make_event(
        &mut state,
        &request.actor_id,
        Some(DocumentId::new(request.document_id.clone())),
        mount_relative_path.clone(),
        relative_path.clone(),
        purge_history_summary(
            request.document_id.as_str(),
            mount_relative_path.as_deref(),
            relative_path.as_deref(),
        ),
        ProvenanceEventKind::DocumentHistoryPurged,
    );
    super::state::save_workspace_state(connection, &state)?;
    append_event(connection, &request.workspace_id, &event)?;
    Ok(())
}

pub(super) fn redact_document_body_history(
    connection: &rusqlite::Transaction<'_>,
    request: &RedactDocumentBodyHistoryRequest,
) -> Result<(), StoreError> {
    let revisions = read_body_revisions(connection, &request.workspace_id)?;
    let mut matched_seqs = Vec::new();
    for revision in revisions {
        if revision.document_id != request.document_id {
            continue;
        }
        let Some(redacted) = revision.redacted(&request.exact_text)? else {
            continue;
        };
        matched_seqs.push(redacted.seq);
        connection.execute(
            "update body_revisions set revision_json = ?3 where workspace_id = ?1 and seq = ?2",
            params![
                request.workspace_id,
                redacted.seq as i64,
                encode_json(&redacted)?
            ],
        )?;
    }
    if matched_seqs.is_empty() {
        return Err(StoreError::new(format!(
            "document {} has no retained body history matching {:?} in workspace {}",
            request.document_id, request.exact_text, request.workspace_id
        )));
    }
    super::super::history::ensure_expected_history_match_set(
        &request.document_id,
        request.expected_match_seqs.as_ref(),
        &matched_seqs,
        "redaction",
    )?;

    let live_path = connection
        .query_row(
            "select mount_path, relative_path from document_paths \
             where workspace_id = ?1 and document_id = ?2 and deleted = false",
            params![request.workspace_id, request.document_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    let mut state = load_required_workspace_state(connection, &request.workspace_id)?;
    let mount_relative_path = live_path.as_ref().map(|(mount, _)| mount.clone());
    let relative_path = live_path.as_ref().map(|(_, relative)| relative.clone());
    let event = make_event(
        &mut state,
        &request.actor_id,
        Some(DocumentId::new(request.document_id.clone())),
        mount_relative_path.clone(),
        relative_path.clone(),
        redact_history_summary(
            request.document_id.as_str(),
            mount_relative_path.as_deref(),
            relative_path.as_deref(),
        ),
        ProvenanceEventKind::DocumentHistoryRedacted,
    );
    super::state::save_workspace_state(connection, &state)?;
    append_event(connection, &request.workspace_id, &event)?;
    Ok(())
}

fn purge_history_summary(
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

fn redact_history_summary(
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
