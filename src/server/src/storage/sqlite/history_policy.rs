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

fn decode_nonzero_policy_component(raw: i64, field: &str) -> Result<u32, StoreError> {
    let value = u32::try_from(raw)
        .map_err(|_| StoreError::new(format!("history compaction {field} must be non-negative")))?;
    if value == 0 {
        return Err(StoreError::new(format!(
            "history compaction {field} must be at least 1"
        )));
    }
    Ok(value)
}

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
        let revisions = decode_nonzero_policy_component(row.get::<_, i64>(1)?, "revisions")
            .map_err(|err| rusqlite::Error::FromSqlConversionFailure(
                1,
                rusqlite::types::Type::Integer,
                Box::new(err),
            ))?;
        let frequency = decode_nonzero_policy_component(row.get::<_, i64>(2)?, "frequency")
            .map_err(|err| rusqlite::Error::FromSqlConversionFailure(
                2,
                rusqlite::types::Type::Integer,
                Box::new(err),
            ))?;
        Ok(StoredHistoryCompactionPolicyOverride {
            repo_relative_path: std::path::PathBuf::from(row.get::<_, String>(0)?),
            policy: HistoryCompactionPolicy {
                revisions,
                frequency,
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
            i64::from(request.policy.revisions),
            i64::from(request.policy.frequency),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlite_policy_loader_rejects_invalid_persisted_values() {
        let connection = Connection::open_in_memory().expect("open sqlite");
        connection
            .execute_batch(
                "create table history_compaction_policies (
                    workspace_id text not null,
                    repo_relative_path text not null,
                    revisions integer not null,
                    frequency integer not null,
                    primary key (workspace_id, repo_relative_path)
                );",
            )
            .expect("create schema");
        connection
            .execute(
                "insert into history_compaction_policies (workspace_id, repo_relative_path, revisions, frequency)
                 values (?1, ?2, ?3, ?4)",
                params!["workspace-1", "private/file.txt", -1_i64, 0_i64],
            )
            .expect("insert invalid row");

        let err = read_history_compaction_policies(&connection, "workspace-1")
            .expect_err("invalid persisted values should fail");
        let message = err.to_string();
        assert!(
            message.contains("history compaction revisions must be non-negative")
                || message.contains("history compaction frequency must be at least 1"),
            "unexpected error: {message}"
        );
    }
}
