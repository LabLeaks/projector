/**
@module PROJECTOR.EDGE.CLI
Parses top-level projector CLI commands and delegates to narrower edge modules for connection management, sync-entry operations, daemon control, and diagnostics.
*/
// @fileimplements PROJECTOR.EDGE.CLI
use std::env;
use std::error::Error;
use std::process;

mod browser_ui;
mod cli_support;
mod connection_cli;
mod daemon_cli;
mod diagnostics_cli;
mod get_browser;
mod restore_browser;
mod sync_entry_cli;

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("sync") => daemon_cli::run_sync_command(args.collect()),
        Some("connect") => connection_cli::run_connect(args.collect()),
        Some("disconnect") => connection_cli::run_disconnect(args.collect()),
        Some("deploy") => connection_cli::run_deploy(args.collect()),
        Some("add") => sync_entry_cli::run_add(args.collect()),
        Some("get") => sync_entry_cli::run_get(args.collect()),
        Some("remove") | Some("rm") => sync_entry_cli::run_remove(args.collect()),
        Some("doctor") => diagnostics_cli::run_doctor(),
        Some("status") => diagnostics_cli::run_status(),
        Some("log") => diagnostics_cli::run_log(),
        Some("history") => diagnostics_cli::run_history(args.collect()),
        Some("restore") => diagnostics_cli::run_restore(args.collect()),
        Some("redact") => diagnostics_cli::run_redact(args.collect()),
        Some("purge") => diagnostics_cli::run_purge(args.collect()),
        Some("daemon") => daemon_cli::run_daemon(args.collect()),
        _ => {
            cli_support::print_usage();
            Ok(())
        }
    }
}
