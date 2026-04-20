/**
@module PROJECTOR.SERVER.FILE_PROVENANCE
Owns file-backed workspace cursor calculation, event persistence, and event listing.
*/
// @fileimplements PROJECTOR.SERVER.FILE_PROVENANCE
use std::fs;
use std::path::Path;

use projector_domain::ProvenanceEvent;

use super::StoreError;
use super::workspaces::workspace_dir;

pub(crate) fn file_read_workspace_events(
    state_dir: &Path,
    workspace_id: &str,
) -> Result<Vec<ProvenanceEvent>, StoreError> {
    let events_path = workspace_dir(state_dir, workspace_id).join("events.json");
    if !events_path.exists() {
        return Ok(Vec::new());
    }

    let content = fs::read(events_path)?;
    serde_json::from_slice(&content).map_err(|err| StoreError::new(err.to_string()))
}

pub(crate) fn file_write_workspace_events(
    state_dir: &Path,
    workspace_id: &str,
    events: &[ProvenanceEvent],
) -> Result<(), StoreError> {
    let workspace_dir = workspace_dir(state_dir, workspace_id);
    fs::create_dir_all(&workspace_dir)?;
    let events_path = workspace_dir.join("events.json");
    let encoded =
        serde_json::to_vec_pretty(events).map_err(|err| StoreError::new(err.to_string()))?;
    fs::write(events_path, encoded)?;
    Ok(())
}

pub(crate) fn file_append_workspace_event(
    state_dir: &Path,
    workspace_id: &str,
    event: ProvenanceEvent,
) -> Result<(), StoreError> {
    let mut events = file_read_workspace_events(state_dir, workspace_id)?;
    events.push(event);
    file_write_workspace_events(state_dir, workspace_id, &events)
}

pub(crate) fn file_extend_workspace_events(
    state_dir: &Path,
    workspace_id: &str,
    new_events: Vec<ProvenanceEvent>,
) -> Result<(), StoreError> {
    let mut events = file_read_workspace_events(state_dir, workspace_id)?;
    events.extend(new_events);
    file_write_workspace_events(state_dir, workspace_id, &events)
}

pub(crate) fn file_workspace_cursor(
    state_dir: &Path,
    workspace_id: &str,
) -> Result<u64, StoreError> {
    Ok(file_read_workspace_events(state_dir, workspace_id)?
        .last()
        .map(|event| event.cursor)
        .unwrap_or_default())
}

pub(crate) fn file_list_events(
    state_dir: &Path,
    workspace_id: &str,
    limit: usize,
) -> Result<Vec<ProvenanceEvent>, StoreError> {
    let mut events = file_read_workspace_events(state_dir, workspace_id)?;
    if events.len() > limit {
        events = events.split_off(events.len() - limit);
    }
    Ok(events)
}
