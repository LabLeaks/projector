/**
@module PROJECTOR.SERVER.SQLITE_WORKSPACES
Owns SQLite workspace bootstrap, sync-entry discovery, and delta reads over persisted workspace snapshots and append-only event rows.
*/
// @fileimplements PROJECTOR.SERVER.SQLITE_WORKSPACES
use std::collections::HashSet;
use std::path::PathBuf;

use projector_domain::{BootstrapSnapshot, SyncEntryKind, SyncEntrySummary};
use rusqlite::{Connection, params};

use super::super::StoreError;
use super::super::bodies::snapshot_subset_for_documents;
use super::history::{read_events_since, read_last_event_timestamp};
use super::state::{
    SqliteWorkspaceMetadata, SqliteWorkspaceState, decode_json, load_workspace_state,
    save_workspace_state,
};

pub(super) fn bootstrap_workspace_tx(
    transaction: &rusqlite::Transaction<'_>,
    workspace_id: &str,
    mounts: &[PathBuf],
    source_repo_name: Option<&str>,
    sync_entry_kind: Option<SyncEntryKind>,
) -> Result<SqliteWorkspaceState, StoreError> {
    let requested_mounts = normalize_mounts(mounts);
    if let Some(mut state) = load_workspace_state(transaction, workspace_id)? {
        if state.metadata.mounts != requested_mounts {
            return Err(StoreError::new(format!(
                "workspace {workspace_id} already bound to different mounts"
            )));
        }
        if state.metadata.source_repo_name.is_none() {
            state.metadata.source_repo_name = source_repo_name.map(str::to_owned);
        }
        if state.metadata.entry_kind.is_none() {
            state.metadata.entry_kind = sync_entry_kind;
        }
        save_workspace_state(transaction, &state)?;
        return Ok(state);
    }

    let state = SqliteWorkspaceState {
        metadata: SqliteWorkspaceMetadata {
            workspace_id: workspace_id.to_owned(),
            mounts: requested_mounts,
            source_repo_name: source_repo_name.map(str::to_owned),
            entry_kind: sync_entry_kind,
        },
        snapshot: BootstrapSnapshot::default(),
        cursor: 0,
    };
    save_workspace_state(transaction, &state)?;
    Ok(state)
}

pub(super) fn changes_since(
    connection: &Connection,
    workspace_id: &str,
    since_cursor: u64,
) -> Result<(BootstrapSnapshot, u64), StoreError> {
    let Some(state) = load_workspace_state(connection, workspace_id)? else {
        return Ok((BootstrapSnapshot::default(), 0));
    };
    if state.cursor <= since_cursor {
        return Ok((BootstrapSnapshot::default(), state.cursor));
    }

    let changed_document_ids = read_events_since(connection, workspace_id, since_cursor)?
        .into_iter()
        .filter_map(|event| event.document_id)
        .collect::<HashSet<_>>();

    Ok((
        snapshot_subset_for_documents(&state.snapshot, &changed_document_ids),
        state.cursor,
    ))
}

pub(super) fn list_sync_entries(
    connection: &Connection,
    limit: usize,
) -> Result<Vec<SyncEntrySummary>, StoreError> {
    let mut stmt = connection.prepare(
        "select id, metadata_json, snapshot_json, cursor from workspaces order by cursor desc, id asc limit ?1",
    )?;
    let rows = stmt.query_map(params![limit as i64], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
        ))
    })?;

    let mut entries = Vec::new();
    for row in rows {
        let (workspace_id, metadata_json, snapshot_json, cursor) = row?;
        let metadata: SqliteWorkspaceMetadata = decode_json(&metadata_json)?;
        let snapshot: BootstrapSnapshot = decode_json(&snapshot_json)?;
        let Some(remote_path) = metadata.mounts.first() else {
            continue;
        };
        let kind = metadata
            .entry_kind
            .clone()
            .unwrap_or_else(|| infer_snapshot_kind(&snapshot));
        entries.push(SyncEntrySummary {
            sync_entry_id: workspace_id.clone(),
            workspace_id: workspace_id.clone(),
            remote_path: remote_path.display().to_string(),
            kind: kind.clone(),
            source_repo_name: metadata.source_repo_name,
            last_updated_ms: if cursor > 0 {
                read_last_event_timestamp(connection, &workspace_id)?
            } else {
                None
            },
            preview: preview_summary(&snapshot, &kind),
        });
    }
    Ok(entries)
}

pub(super) fn normalize_mounts(mounts: &[PathBuf]) -> Vec<PathBuf> {
    let mut normalized = mounts.to_vec();
    normalized.sort();
    normalized.dedup();
    normalized
}

fn infer_snapshot_kind(snapshot: &BootstrapSnapshot) -> SyncEntryKind {
    if snapshot
        .manifest
        .entries
        .iter()
        .any(|entry| !entry.deleted && entry.relative_path.as_os_str().is_empty())
    {
        SyncEntryKind::File
    } else {
        SyncEntryKind::Directory
    }
}

fn preview_summary(snapshot: &BootstrapSnapshot, kind: &SyncEntryKind) -> Option<String> {
    match kind {
        SyncEntryKind::File => snapshot.bodies.first().map(|body| {
            body.text
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(120)
                .collect::<String>()
        }),
        SyncEntryKind::Directory => {
            let live_count = snapshot
                .manifest
                .entries
                .iter()
                .filter(|entry| !entry.deleted)
                .count();
            if live_count == 0 {
                None
            } else if let Some(first_entry) = snapshot
                .manifest
                .entries
                .iter()
                .filter(|entry| !entry.deleted)
                .min_by(|left, right| left.relative_path.cmp(&right.relative_path))
            {
                Some(format!(
                    "{live_count} files; first={}",
                    first_entry.relative_path.display()
                ))
            } else {
                Some(format!("{live_count} files"))
            }
        }
    }
}
