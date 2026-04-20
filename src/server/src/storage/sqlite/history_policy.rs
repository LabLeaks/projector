/**
@module PROJECTOR.SERVER.SQLITE_HISTORY_POLICY
Owns SQLite history compaction policy persistence and enforcement over retained body revisions.
*/
// @fileimplements PROJECTOR.SERVER.SQLITE_HISTORY_POLICY
use projector_domain::{
    ClearHistoryCompactionPolicyRequest, GetHistoryCompactionPolicyResponse,
    HistoryCompactionPolicy, SetHistoryCompactionPolicyRequest,
};
use rusqlite::{Connection, params};

use super::super::super::StoreError;
use super::super::super::history_compaction::{
    StoredHistoryCompactionPolicyOverride, compact_document_body_revisions,
    history_compaction_response, normalize_history_compaction_path,
    resolve_history_compaction_policy, validate_history_compaction_policy,
};
use super::super::state::{encode_json, load_required_workspace_state};
use super::revisions::read_body_revisions;

fn read_history_compaction_policies(
    connection: &Connection,
    workspace_id: &str,
) -> Result<Vec<StoredHistoryCompactionPolicyOverride>, StoreError> {
    let mut stmt = connection.prepare(
        "select repo_relative_path, revisions, frequency \
         from history_compaction_policies \
         where workspace_id = ?1 \
         order by repo_relative_path asc",
    )?;
    let rows = stmt.query_map(params![workspace_id], |row| {
        Ok(StoredHistoryCompactionPolicyOverride {
            repo_relative_path: std::path::PathBuf::from(row.get::<_, String>(0)?),
            policy: HistoryCompactionPolicy {
                revisions: row.get::<_, i64>(1)? as usize,
                frequency: row.get::<_, i64>(2)? as usize,
            },
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(StoreError::from)
}

pub(crate) fn get_history_compaction_policy(
    connection: &Connection,
    workspace_id: &str,
    repo_relative_path: &str,
) -> Result<GetHistoryCompactionPolicyResponse, StoreError> {
    let normalized_path = normalize_history_compaction_path(repo_relative_path)?;
    Ok(history_compaction_response(
        &read_history_compaction_policies(connection, workspace_id)?,
        &normalized_path,
    ))
}

pub(crate) fn set_history_compaction_policy(
    connection: &rusqlite::Transaction<'_>,
    request: &SetHistoryCompactionPolicyRequest,
) -> Result<(), StoreError> {
    validate_history_compaction_policy(&request.policy)?;
    let normalized_path = normalize_history_compaction_path(&request.repo_relative_path)?;
    connection.execute(
        "insert into history_compaction_policies (workspace_id, repo_relative_path, revisions, frequency) \
         values (?1, ?2, ?3, ?4) \
         on conflict(workspace_id, repo_relative_path) do update set \
           revisions = excluded.revisions, \
           frequency = excluded.frequency",
        params![
            request.workspace_id,
            normalized_path.display().to_string(),
            request.policy.revisions as i64,
            request.policy.frequency as i64,
        ],
    )?;
    Ok(())
}

pub(crate) fn clear_history_compaction_policy(
    connection: &rusqlite::Transaction<'_>,
    request: &ClearHistoryCompactionPolicyRequest,
) -> Result<bool, StoreError> {
    let normalized_path = normalize_history_compaction_path(&request.repo_relative_path)?;
    Ok(connection.execute(
        "delete from history_compaction_policies where workspace_id = ?1 and repo_relative_path = ?2",
        params![request.workspace_id, normalized_path.display().to_string()],
    )? > 0)
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
    let resolved = resolve_history_compaction_policy(
        &read_history_compaction_policies(connection, workspace_id)?,
        &repo_relative_path,
    );
    let original = read_body_revisions(connection, workspace_id)?
        .into_iter()
        .filter(|revision| revision.document_id == document_id)
        .collect::<Vec<_>>();
    let compacted = compact_document_body_revisions(&original, document_id, &resolved.policy)?;
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
