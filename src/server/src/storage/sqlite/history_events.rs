/**
@module PROJECTOR.SERVER.SQLITE_EVENT_HISTORY
Owns SQLite event history reads for incremental sync changes and observability surfaces.
*/
// @fileimplements PROJECTOR.SERVER.SQLITE_EVENT_HISTORY
use projector_domain::ProvenanceEvent;
use rusqlite::{Connection, OptionalExtension, params};

use super::super::StoreError;
use super::super::state::decode_json;

pub(crate) fn read_events_since(
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

pub(crate) fn read_recent_events(
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

pub(crate) fn read_last_event_timestamp(
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
