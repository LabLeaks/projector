/**
@module PROJECTOR.SERVER.PROVENANCE
Owns workspace cursors, event persistence, event listing, and synthetic provenance generation for file-backed and Postgres-backed stores.
*/
// @fileimplements PROJECTOR.SERVER.PROVENANCE
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use projector_domain::{
    BootstrapSnapshot, DocumentId, ManifestEntry, ProvenanceEvent, ProvenanceEventKind,
};
use tokio_postgres::Client;

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

pub(crate) async fn current_workspace_cursor_tx(
    transaction: &tokio_postgres::Transaction<'_>,
    workspace_id: &str,
) -> Result<u64, StoreError> {
    Ok(transaction
        .query_one(
            "select coalesce(max(seq), 0)::bigint as cursor from provenance_events where workspace_id = $1",
            &[&workspace_id],
        )
        .await?
        .get::<_, i64>("cursor") as u64)
}

pub(crate) async fn insert_event_tx(
    transaction: &tokio_postgres::Transaction<'_>,
    workspace_id: &str,
    actor_id: &str,
    document_id: Option<&str>,
    mount_path: Option<&str>,
    relative_path: Option<&str>,
    kind: ProvenanceEventKind,
    summary: &str,
) -> Result<u64, StoreError> {
    let kind_value = event_kind_db_value(&kind);
    let row = transaction
        .query_one(
            "insert into provenance_events \
             (workspace_id, actor_id, document_id, mount_path, relative_path, event_kind, summary) \
             values ($1, $2, $3, $4, $5, $6, $7) \
             returning seq",
            &[
                &workspace_id,
                &actor_id,
                &document_id,
                &mount_path,
                &relative_path,
                &kind_value,
                &summary,
            ],
        )
        .await?;
    Ok(row.get::<_, i64>("seq") as u64)
}

pub(crate) async fn postgres_list_events(
    client: &Client,
    workspace_id: &str,
    limit: usize,
) -> Result<Vec<ProvenanceEvent>, StoreError> {
    let rows = client
        .query(
            "select \
                seq, \
                actor_id, \
                document_id, \
                mount_path, \
                relative_path, \
                event_kind, \
                summary, \
                (extract(epoch from created_at) * 1000)::bigint as timestamp_ms \
             from provenance_events \
             where workspace_id = $1 \
             order by seq desc \
             limit $2",
            &[&workspace_id, &(limit as i64)],
        )
        .await?;

    let mut events = rows
        .into_iter()
        .map(|row| {
            Ok(ProvenanceEvent {
                cursor: row.get::<_, i64>("seq") as u64,
                timestamp_ms: row.get::<_, i64>("timestamp_ms") as u128,
                actor_id: projector_domain::ActorId::new(row.get::<_, String>("actor_id")),
                document_id: row
                    .get::<_, Option<String>>("document_id")
                    .map(DocumentId::new),
                mount_relative_path: row.get::<_, Option<String>>("mount_path"),
                relative_path: row.get::<_, Option<String>>("relative_path"),
                summary: row.get::<_, String>("summary"),
                kind: parse_event_kind(&row.get::<_, String>("event_kind"))?,
            })
        })
        .collect::<Result<Vec<_>, StoreError>>()?;
    events.reverse();
    Ok(events)
}

pub(crate) fn parse_event_kind(raw: &str) -> Result<ProvenanceEventKind, StoreError> {
    match raw {
        "document_created" => Ok(ProvenanceEventKind::DocumentCreated),
        "document_moved" => Ok(ProvenanceEventKind::DocumentMoved),
        "document_updated" => Ok(ProvenanceEventKind::DocumentUpdated),
        "document_deleted" => Ok(ProvenanceEventKind::DocumentDeleted),
        "document_history_redacted" => Ok(ProvenanceEventKind::DocumentHistoryRedacted),
        "document_history_purged" => Ok(ProvenanceEventKind::DocumentHistoryPurged),
        "sync_bootstrapped" => Ok(ProvenanceEventKind::SyncBootstrapped),
        "sync_reused_binding" => Ok(ProvenanceEventKind::SyncReusedBinding),
        "sync_recovery" => Ok(ProvenanceEventKind::SyncRecovery),
        "sync_issue" => Ok(ProvenanceEventKind::SyncIssue),
        other => Err(StoreError::new(format!("unknown event kind {other}"))),
    }
}

pub(crate) fn synthetic_events_for_snapshot_change(
    previous: &BootstrapSnapshot,
    current: &BootstrapSnapshot,
    starting_cursor: u64,
) -> Vec<ProvenanceEvent> {
    let previous_entries = previous
        .manifest
        .entries
        .iter()
        .map(|entry| (entry.document_id.clone(), entry))
        .collect::<HashMap<_, _>>();
    let previous_bodies = previous
        .bodies
        .iter()
        .map(|body| (body.document_id.clone(), body.text.as_str()))
        .collect::<HashMap<_, _>>();

    let mut events = Vec::new();
    let mut cursor = starting_cursor;
    for entry in &current.manifest.entries {
        let previous_entry = previous_entries.get(&entry.document_id);
        let current_body = current
            .bodies
            .iter()
            .find(|body| body.document_id == entry.document_id)
            .map(|body| body.text.as_str())
            .unwrap_or("");

        let kind = match previous_entry {
            None if !entry.deleted => Some(ProvenanceEventKind::DocumentCreated),
            Some(previous_entry) if !previous_entry.deleted && entry.deleted => {
                Some(ProvenanceEventKind::DocumentDeleted)
            }
            Some(previous_entry) if !entry.deleted && !paths_match(previous_entry, entry) => {
                Some(ProvenanceEventKind::DocumentMoved)
            }
            Some(_previous_entry)
                if !entry.deleted
                    && (previous_bodies
                        .get(&entry.document_id)
                        .copied()
                        .unwrap_or_default()
                        != current_body) =>
            {
                Some(ProvenanceEventKind::DocumentUpdated)
            }
            _ => None,
        };

        if let Some(kind) = kind {
            cursor += 1;
            events.push(ProvenanceEvent {
                cursor,
                timestamp_ms: now_ms(),
                actor_id: projector_domain::ActorId::new("server-seed"),
                document_id: Some(entry.document_id.clone()),
                mount_relative_path: Some(entry.mount_relative_path.display().to_string()),
                relative_path: Some(entry.relative_path.display().to_string()),
                summary: synthetic_event_summary(&kind, entry),
                kind,
            });
        }
    }

    events
}

fn event_kind_db_value(kind: &ProvenanceEventKind) -> &'static str {
    match kind {
        ProvenanceEventKind::DocumentCreated => "document_created",
        ProvenanceEventKind::DocumentMoved => "document_moved",
        ProvenanceEventKind::DocumentUpdated => "document_updated",
        ProvenanceEventKind::DocumentDeleted => "document_deleted",
        ProvenanceEventKind::DocumentHistoryRedacted => "document_history_redacted",
        ProvenanceEventKind::DocumentHistoryPurged => "document_history_purged",
        ProvenanceEventKind::SyncBootstrapped => "sync_bootstrapped",
        ProvenanceEventKind::SyncReusedBinding => "sync_reused_binding",
        ProvenanceEventKind::SyncRecovery => "sync_recovery",
        ProvenanceEventKind::SyncIssue => "sync_issue",
    }
}

fn paths_match(left: &ManifestEntry, right: &ManifestEntry) -> bool {
    left.mount_relative_path == right.mount_relative_path
        && left.relative_path == right.relative_path
}

fn display_manifest_path(entry: &ManifestEntry) -> String {
    if entry.relative_path.as_os_str().is_empty() {
        entry.mount_relative_path.display().to_string()
    } else {
        entry
            .mount_relative_path
            .join(&entry.relative_path)
            .display()
            .to_string()
    }
}

fn synthetic_event_summary(kind: &ProvenanceEventKind, entry: &ManifestEntry) -> String {
    match kind {
        ProvenanceEventKind::DocumentCreated => {
            format!("created text document at {}", display_manifest_path(entry))
        }
        ProvenanceEventKind::DocumentMoved => {
            format!("moved text document to {}", display_manifest_path(entry))
        }
        ProvenanceEventKind::DocumentUpdated => {
            format!("updated text document at {}", display_manifest_path(entry))
        }
        ProvenanceEventKind::DocumentDeleted => {
            format!("deleted text document at {}", display_manifest_path(entry))
        }
        ProvenanceEventKind::DocumentHistoryRedacted => {
            format!("redacted retained body history for {}", display_manifest_path(entry))
        }
        ProvenanceEventKind::DocumentHistoryPurged => {
            format!("purged retained body history for {}", display_manifest_path(entry))
        }
        ProvenanceEventKind::SyncBootstrapped => "bootstrapped local projector state".to_owned(),
        ProvenanceEventKind::SyncReusedBinding => "reused existing checkout binding".to_owned(),
        ProvenanceEventKind::SyncRecovery => "recorded local sync recovery action".to_owned(),
        ProvenanceEventKind::SyncIssue => "recorded local sync issue".to_owned(),
    }
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time before unix epoch")
        .as_millis()
}
