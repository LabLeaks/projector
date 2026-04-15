/**
@module PROJECTOR.EDGE.LOG_LOCAL
Owns local audit-log filtering and rendering for sync issues, sync recovery, and fallback local provenance output.
*/
// @fileimplements PROJECTOR.EDGE.LOG_LOCAL
use std::error::Error;

use projector_domain::ProvenanceEventKind;
use projector_runtime::FileProvenanceLog;

use crate::cli_support::format_kind;

pub(super) fn print_local_overlay_events(
    local_log: &FileProvenanceLog,
) -> Result<bool, Box<dyn Error>> {
    let local_overlay_events = local_log
        .read_all()?
        .into_iter()
        .filter(|event| {
            matches!(
                event.kind,
                ProvenanceEventKind::SyncIssue | ProvenanceEventKind::SyncRecovery
            )
        })
        .collect::<Vec<_>>();

    let mut printed = false;
    for event in &local_overlay_events {
        println!(
            "{} actor={} kind={} path={} summary={}",
            event.timestamp_ms,
            event.actor_id.as_str(),
            format_kind(&event.kind),
            event.path,
            event.summary
        );
        printed = true;
    }
    Ok(printed)
}

pub(super) fn print_all_local_events(
    local_log: &FileProvenanceLog,
    printed_overlay: bool,
) -> Result<(), Box<dyn Error>> {
    let events = local_log.read_all()?;
    if events.is_empty() && !printed_overlay {
        println!("no events");
        return Ok(());
    }

    for event in events {
        println!(
            "{} actor={} kind={} path={} summary={}",
            event.timestamp_ms,
            event.actor_id.as_str(),
            format_kind(&event.kind),
            event.path,
            event.summary
        );
    }
    Ok(())
}
