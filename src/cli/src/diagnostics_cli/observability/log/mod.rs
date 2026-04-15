/**
@module PROJECTOR.EDGE.LOG_CLI
Coordinates the operational audit log surface by delegating local overlay rendering and remote provenance reads to narrower log helpers.
*/
// @fileimplements PROJECTOR.EDGE.LOG_CLI
use std::error::Error;

use projector_runtime::FileProvenanceLog;

use crate::cli_support::repo_root;

mod local;
mod remote;

use local::{print_all_local_events, print_local_overlay_events};
use remote::{fetch_remote_events, print_remote_events};

pub(crate) fn run_log() -> Result<(), Box<dyn Error>> {
    let repo_root = repo_root()?;
    let local_log = FileProvenanceLog::new(repo_root.join(".projector/events.log"));
    let printed_overlay = print_local_overlay_events(&local_log)?;

    let remote_events = fetch_remote_events(&repo_root)?;
    if !remote_events.is_empty() {
        print_remote_events(remote_events);
        return Ok(());
    }

    print_all_local_events(&local_log, printed_overlay)
}
