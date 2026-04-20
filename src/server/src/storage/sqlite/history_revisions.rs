/**
@module PROJECTOR.SERVER.SQLITE_REVISION_HISTORY
Owns SQLite body and path revision reads, listing, preview, and historical path resolution.
*/
// @fileimplements PROJECTOR.SERVER.SQLITE_REVISION_HISTORY
use projector_domain::{
    DocumentBodyPurgeMatch, DocumentBodyRedactionMatch, DocumentBodyRevision, DocumentId,
    DocumentPathRevision, PreviewPurgeDocumentBodyHistoryRequest,
    PreviewRedactDocumentBodyHistoryRequest,
};
use rusqlite::{Connection, params};

use super::super::super::StoreError;
pub(super) use super::super::super::history::{FileBodyRevision, FilePathRevision};
use super::super::super::history_surgery::{retained_purge_matches, retained_redaction_matches};
use super::super::state::{decode_json, effective_workspace_cursor};

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

pub(crate) fn read_path_history(
    connection: &Connection,
    workspace_id: &str,
) -> Result<Vec<FilePathRevision>, StoreError> {
    let mut stmt = connection.prepare(
        "select revision_json from path_revisions where workspace_id = ?1 order by seq asc",
    )?;
    let rows = stmt.query_map(params![workspace_id], |row| row.get::<_, String>(0))?;
    rows.map(|row| decode_json(&row?)).collect()
}

pub(crate) fn list_body_revisions(
    connection: &Connection,
    workspace_id: &str,
    document_id: &str,
    limit: usize,
) -> Result<Vec<DocumentBodyRevision>, StoreError> {
    let mut revisions = read_body_revisions(connection, workspace_id)?
        .into_iter()
        .filter(|revision| revision.document_id == document_id)
        .map(|revision: FileBodyRevision| revision.to_public_revision())
        .collect::<Vec<_>>();
    if revisions.len() > limit {
        revisions = revisions.split_off(revisions.len() - limit);
    }
    Ok(revisions)
}

pub(crate) fn preview_redact_document_body_history(
    connection: &Connection,
    request: &PreviewRedactDocumentBodyHistoryRequest,
) -> Result<Vec<DocumentBodyRedactionMatch>, StoreError> {
    let matches = retained_redaction_matches(
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

pub(crate) fn preview_purge_document_body_history(
    connection: &Connection,
    request: &PreviewPurgeDocumentBodyHistoryRequest,
) -> Result<Vec<DocumentBodyPurgeMatch>, StoreError> {
    let matches = retained_purge_matches(
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

pub(crate) fn list_path_revisions(
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

pub(crate) fn resolve_document_by_historical_path(
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
