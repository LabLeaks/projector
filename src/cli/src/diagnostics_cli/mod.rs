/**
@module PROJECTOR.EDGE.DIAGNOSTICS_CLI
Owns operational status, doctor, log, history, and restore surfaces, including the emergency history and PITR flows exposed through the CLI.
*/
// @fileimplements PROJECTOR.EDGE.DIAGNOSTICS_CLI
mod history_restore;
mod observability;

pub(crate) use history_restore::{
    resolve_live_entry_for_repo_relative_path, run_history, run_purge, run_redact, run_restore,
};
pub(crate) use observability::{run_doctor, run_log, run_status};
