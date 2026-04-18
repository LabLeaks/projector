/**
@module PROJECTOR.SERVER.SQLITE_STATE
Owns the SQLite schema, workspace row persistence, append-only row writes, and shared encode or id helpers for the SQLite server store.
*/
// @fileimplements PROJECTOR.SERVER.SQLITE_STATE
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use super::super::body_state::CanonicalBodyState;
use projector_domain::{
    BootstrapSnapshot, DocumentId, ProvenanceEvent, ProvenanceEventKind, SyncEntryKind,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::super::StoreError;

pub(super) const SQLITE_SCHEMA: &str = r#"
create table if not exists workspaces (
  id text primary key,
  metadata_json text not null,
  snapshot_json text not null,
  cursor integer not null default 0
);

create table if not exists events (
  workspace_id text not null,
  seq integer not null,
  event_json text not null,
  primary key (workspace_id, seq)
);

create table if not exists body_revisions (
  workspace_id text not null,
  seq integer not null,
  revision_json text not null,
  primary key (workspace_id, seq)
);

create table if not exists path_revisions (
  workspace_id text not null,
  seq integer not null,
  revision_json text not null,
  primary key (workspace_id, seq)
);
"#;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(super) struct SqliteWorkspaceMetadata {
    pub(super) workspace_id: String,
    pub(super) mounts: Vec<PathBuf>,
    pub(super) source_repo_name: Option<String>,
    pub(super) entry_kind: Option<SyncEntryKind>,
}

#[derive(Clone, Debug)]
pub(super) struct SqliteWorkspaceState {
    pub(super) metadata: SqliteWorkspaceMetadata,
    pub(super) snapshot: BootstrapSnapshot,
    pub(super) cursor: u64,
}

pub(super) fn load_workspace_state(
    connection: &Connection,
    workspace_id: &str,
) -> Result<Option<SqliteWorkspaceState>, StoreError> {
    connection
        .query_row(
            "select metadata_json, snapshot_json, cursor from workspaces where id = ?1",
            params![workspace_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()?
        .map(|(metadata_json, snapshot_json, cursor)| {
            Ok(SqliteWorkspaceState {
                metadata: decode_json(&metadata_json)?,
                snapshot: decode_json(&snapshot_json)?,
                cursor: cursor as u64,
            })
        })
        .transpose()
}

pub(super) fn load_required_workspace_state(
    connection: &Connection,
    workspace_id: &str,
) -> Result<SqliteWorkspaceState, StoreError> {
    load_workspace_state(connection, workspace_id)?
        .ok_or_else(|| StoreError::new(format!("workspace {workspace_id} is not bound")))
}

pub(super) fn save_workspace_state(
    connection: &Connection,
    state: &SqliteWorkspaceState,
) -> Result<(), StoreError> {
    let metadata_json = encode_json(&state.metadata)?;
    let snapshot_json = encode_json(&state.snapshot)?;
    connection.execute(
        "insert into workspaces (id, metadata_json, snapshot_json, cursor) values (?1, ?2, ?3, ?4)
         on conflict(id) do update set metadata_json = excluded.metadata_json, snapshot_json = excluded.snapshot_json, cursor = excluded.cursor",
        params![
            state.metadata.workspace_id,
            metadata_json,
            snapshot_json,
            state.cursor as i64
        ],
    )?;
    Ok(())
}

pub(super) fn append_event(
    connection: &Connection,
    workspace_id: &str,
    event: &ProvenanceEvent,
) -> Result<(), StoreError> {
    connection.execute(
        "insert into events (workspace_id, seq, event_json) values (?1, ?2, ?3)",
        params![workspace_id, event.cursor as i64, encode_json(event)?],
    )?;
    Ok(())
}

pub(super) fn append_body_revision(
    connection: &Connection,
    workspace_id: &str,
    revision: &super::super::history::FileBodyRevision,
) -> Result<(), StoreError> {
    connection.execute(
        "insert into body_revisions (workspace_id, seq, revision_json) values (?1, ?2, ?3)",
        params![workspace_id, revision.seq as i64, encode_json(revision)?],
    )?;
    Ok(())
}

pub(super) fn append_path_revision(
    connection: &Connection,
    workspace_id: &str,
    revision: &super::super::history::FilePathRevision,
) -> Result<(), StoreError> {
    connection.execute(
        "insert into path_revisions (workspace_id, seq, revision_json) values (?1, ?2, ?3)",
        params![workspace_id, revision.seq as i64, encode_json(revision)?],
    )?;
    Ok(())
}

pub(super) fn make_event(
    state: &mut SqliteWorkspaceState,
    actor_id: &str,
    document_id: Option<DocumentId>,
    mount_relative_path: Option<String>,
    relative_path: Option<String>,
    summary: String,
    kind: ProvenanceEventKind,
) -> ProvenanceEvent {
    state.cursor += 1;
    ProvenanceEvent {
        cursor: state.cursor,
        timestamp_ms: now_ms(),
        actor_id: projector_domain::ActorId::new(actor_id.to_owned()),
        document_id,
        mount_relative_path,
        relative_path,
        summary,
        kind,
    }
}

pub(super) fn upsert_body_state(
    snapshot: &mut BootstrapSnapshot,
    document_id: &DocumentId,
    state: &CanonicalBodyState,
) {
    if let Some(body) = snapshot
        .bodies
        .iter_mut()
        .find(|body| body.document_id == *document_id)
    {
        body.text = state.materialized_text().to_owned();
    } else {
        snapshot
            .bodies
            .push(state.clone().into_document_body(document_id.clone()));
    }
}

pub(super) fn display_document_path(mount_path: &str, relative_path: &str) -> String {
    if relative_path.is_empty() {
        mount_path.to_owned()
    } else {
        format!("{mount_path}/{relative_path}")
    }
}

pub(super) fn effective_workspace_cursor(seq: u64, workspace_cursor: u64) -> u64 {
    if workspace_cursor == 0 {
        seq
    } else {
        workspace_cursor
    }
}

pub(super) fn encode_json<T: Serialize>(value: &T) -> Result<String, StoreError> {
    serde_json::to_string(value).map_err(|err| StoreError::new(err.to_string()))
}

pub(super) fn decode_json<T: for<'de> Deserialize<'de>>(value: &str) -> Result<T, StoreError> {
    serde_json::from_str(value).map_err(|err| StoreError::new(err.to_string()))
}

pub(super) fn make_document_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time before unix epoch")
        .as_nanos();
    format!("doc-{nanos}")
}

pub(super) fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time before unix epoch")
        .as_millis()
}
