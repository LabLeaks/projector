/**
@module PROJECTOR.SERVER.SQLITE_HISTORY_SURGERY
Owns SQLite retained history purge and redaction over body revisions, including audit summary generation.
*/
// @fileimplements PROJECTOR.SERVER.SQLITE_HISTORY_SURGERY
use projector_domain::{
    DocumentId, ProvenanceEventKind, PurgeDocumentBodyHistoryRequest,
    RedactDocumentBodyHistoryRequest,
};
use rusqlite::{OptionalExtension, params};

use super::super::super::StoreError;
use super::super::super::history_surgery::ensure_expected_history_match_set;
use super::super::state::{
    append_event, encode_json, load_required_workspace_state, make_event, save_workspace_state,
};
use super::revisions::read_body_revisions;

pub(crate) fn purge_document_body_history(
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
    ensure_expected_history_match_set(
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
    save_workspace_state(connection, &state)?;
    append_event(connection, &request.workspace_id, &event)?;
    Ok(())
}

pub(crate) fn redact_document_body_history(
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
    ensure_expected_history_match_set(
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
    save_workspace_state(connection, &state)?;
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
