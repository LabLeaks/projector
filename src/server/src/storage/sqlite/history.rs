/**
@module PROJECTOR.SERVER.SQLITE_HISTORY
Owns SQLite event and revision reads for list, discovery, and historical path resolution over append-only server history rows.
*/
// @fileimplements PROJECTOR.SERVER.SQLITE_HISTORY
use projector_domain::{
    DocumentBodyRedactionMatch, DocumentBodyRevision, DocumentId, DocumentPathRevision,
    PreviewRedactDocumentBodyHistoryRequest, ProvenanceEvent, ProvenanceEventKind,
    PurgeDocumentBodyHistoryRequest, RedactDocumentBodyHistoryRequest,
};
use rusqlite::{Connection, OptionalExtension, params};

use super::super::StoreError;
pub(super) use super::super::history::{FileBodyRevision, FilePathRevision};
use super::state::{
    append_event, decode_json, effective_workspace_cursor, encode_json,
    load_required_workspace_state, make_event,
};

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
    let matched = connection.execute(
        "update body_revisions \
         set revision_json = json_set(revision_json, '$.base_text', '', '$.body_text', '') \
         where workspace_id = ?1 and json_extract(revision_json, '$.document_id') = ?2",
        params![request.workspace_id, request.document_id],
    )?;
    if matched == 0 {
        return Err(StoreError::new(format!(
            "document {} has no retained body history in workspace {}",
            request.document_id, request.workspace_id
        )));
    }

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
    let mut matched = 0usize;
    for revision in revisions {
        if revision.document_id != request.document_id {
            continue;
        }
        let Some(redacted) = revision.redacted(&request.exact_text)? else {
            continue;
        };
        connection.execute(
            "update body_revisions set revision_json = ?3 where workspace_id = ?1 and seq = ?2",
            params![
                request.workspace_id,
                redacted.seq as i64,
                encode_json(&redacted)?
            ],
        )?;
        matched += 1;
    }
    if matched == 0 {
        return Err(StoreError::new(format!(
            "document {} has no retained body history matching {:?} in workspace {}",
            request.document_id, request.exact_text, request.workspace_id
        )));
    }

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
