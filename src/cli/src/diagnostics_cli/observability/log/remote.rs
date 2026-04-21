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

pub(super) fn print_remote_fetch_failure(error: &dyn Error) {
    for line in remote_fetch_failure_lines(error) {
        println!("{line}");
    }
}

pub(super) fn is_reportable_remote_fetch_failure(error: &dyn Error) -> bool {
    classify_remote_fetch_failure(&error_chain_detail(error)).is_some()
}

fn remote_fetch_failure_lines(error: &dyn Error) -> [String; 3] {
    let detail = error_chain_detail(error);
    let kind =
        classify_remote_fetch_failure(&detail).unwrap_or(RemoteFetchFailureKind::TransportFailure);
    [
        format!("log_remote_access: {}", kind.label()),
        format!("log_remote_error: {detail}"),
        "log_remote_hint: remote log fetch failed from this runtime; this does not prove daemon or server sync failure".to_owned(),
    ]
}

fn error_chain_detail(error: &dyn Error) -> String {
    let mut messages = vec![error.to_string()];
    let mut source = error.source();
    while let Some(error) = source {
        messages.push(error.to_string());
        source = error.source();
    }
    messages.join(": ")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RemoteFetchFailureKind {
    LikelyLocalRuntimeRestriction,
    TransportFailure,
}

impl RemoteFetchFailureKind {
    fn label(self) -> &'static str {
        match self {
            Self::LikelyLocalRuntimeRestriction => "blocked_by_local_runtime",
            Self::TransportFailure => "transport_failed_from_this_runtime",
        }
    }
}

fn classify_remote_fetch_failure(detail: &str) -> Option<RemoteFetchFailureKind> {
    let detail = detail.to_ascii_lowercase();
    let restriction_markers = [
        "operation not permitted",
        "permission denied",
        "dns error",
        "failed to lookup address information",
        "temporary failure in name resolution",
        "could not resolve host",
        "network is unreachable",
        "no route to host",
    ];
    if restriction_markers
        .iter()
        .any(|marker| detail.contains(marker))
    {
        return Some(RemoteFetchFailureKind::LikelyLocalRuntimeRestriction);
    }

    let transport_markers = [
        "connection refused",
        "connection reset",
        "connection closed",
        "connection aborted",
        "timed out",
        "timeout",
        "failed to connect",
        "error sending request",
    ];
    if transport_markers
        .iter()
        .any(|marker| detail.contains(marker))
    {
        Some(RemoteFetchFailureKind::TransportFailure)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::{
        RemoteFetchFailureKind, classify_remote_fetch_failure, is_reportable_remote_fetch_failure,
        remote_fetch_failure_lines,
    };

    // @verifies PROJECTOR.CLI.LOG.DISTINGUISHES_LOCAL_TRANSPORT_RESTRICTIONS
    #[test]
    fn classifies_permission_denied_as_local_runtime_restriction() {
        let lines = remote_fetch_failure_lines(&io::Error::new(
            io::ErrorKind::PermissionDenied,
            "operation not permitted",
        ));

        assert_eq!(lines[0], "log_remote_access: blocked_by_local_runtime");
        assert!(lines[1].contains("operation not permitted"));
        assert!(lines[2].contains("does not prove daemon or server sync failure"));
    }

    #[test]
    fn classifies_other_transport_failures_separately() {
        assert_eq!(
            classify_remote_fetch_failure("connection refused"),
            Some(RemoteFetchFailureKind::TransportFailure)
        );
    }

    #[test]
    fn leaves_sync_setup_errors_fatal() {
        let error = io::Error::new(
            io::ErrorKind::InvalidData,
            "server profile homebox is not registered",
        );

        assert!(!is_reportable_remote_fetch_failure(&error));
    }
}
