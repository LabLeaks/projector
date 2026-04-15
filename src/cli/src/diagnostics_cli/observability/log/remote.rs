/**
@module PROJECTOR.EDGE.LOG_REMOTE
Owns remote provenance reads, deduplication, and terminal rendering for the log command.
*/
// @fileimplements PROJECTOR.EDGE.LOG_REMOTE
use std::error::Error;
use std::path::Path;

use projector_domain::ProvenanceEvent;
use projector_runtime::{HttpTransport, Transport};

use crate::cli_support::format_kind;
use crate::diagnostics_cli::history_restore::format_event_path;
use crate::sync_entry_cli::{group_sync_targets_by_workspace, load_sync_targets_with_profiles};

pub(super) fn fetch_remote_events(
    repo_root: &Path,
) -> Result<Vec<ProvenanceEvent>, Box<dyn Error>> {
    let sync_targets = load_sync_targets_with_profiles(repo_root)?;
    let mut remote_events = Vec::new();
    for binding in group_sync_targets_by_workspace(&sync_targets) {
        let Some(server_addr) = binding.server_addr.as_deref() else {
            continue;
        };
        if server_addr == "none" || server_addr.is_empty() {
            continue;
        }

        let mut transport = HttpTransport::new(format!("http://{server_addr}"));
        remote_events.extend(transport.provenance(&binding, 100)?);
    }
    remote_events.sort_by_key(|event| event.timestamp_ms);
    remote_events.dedup_by(|left, right| {
        left.timestamp_ms == right.timestamp_ms
            && left.actor_id == right.actor_id
            && left.kind == right.kind
            && left.document_id == right.document_id
            && left.mount_relative_path == right.mount_relative_path
            && left.relative_path == right.relative_path
            && left.summary == right.summary
    });
    Ok(remote_events)
}

pub(super) fn print_remote_events(remote_events: Vec<ProvenanceEvent>) {
    for event in remote_events {
        println!(
            "{} actor={} kind={} document_id={} path={} summary={}",
            event.timestamp_ms,
            event.actor_id.as_str(),
            format_kind(&event.kind),
            event
                .document_id
                .as_ref()
                .map(|document_id| document_id.as_str())
                .unwrap_or("none"),
            format_event_path(
                event.mount_relative_path.as_deref(),
                event.relative_path.as_deref()
            ),
            event.summary
        );
    }
}
