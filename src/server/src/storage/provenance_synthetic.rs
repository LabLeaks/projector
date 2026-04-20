/**
@module PROJECTOR.SERVER.SYNTHETIC_PROVENANCE
Owns synthetic provenance derivation from bootstrap snapshot changes.
*/
// @fileimplements PROJECTOR.SERVER.SYNTHETIC_PROVENANCE
use std::collections::HashMap;

use projector_domain::{BootstrapSnapshot, ManifestEntry, ProvenanceEvent, ProvenanceEventKind};

use super::provenance::now_ms;

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
            format!(
                "redacted retained body history for {}",
                display_manifest_path(entry)
            )
        }
        ProvenanceEventKind::DocumentHistoryPurged => {
            format!(
                "purged retained body history for {}",
                display_manifest_path(entry)
            )
        }
        ProvenanceEventKind::SyncBootstrapped => "bootstrapped local projector state".to_owned(),
        ProvenanceEventKind::SyncReusedBinding => "reused existing checkout binding".to_owned(),
        ProvenanceEventKind::SyncRecovery => "recorded local sync recovery action".to_owned(),
        ProvenanceEventKind::SyncIssue => "recorded local sync issue".to_owned(),
    }
}
