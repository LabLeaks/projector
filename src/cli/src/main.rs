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
        Some("help") => {
            let command = args.next();
            if let Some(extra) = args.next() {
                return Err(format!(
                    "help accepts at most one command; unexpected argument: {extra}"
                )
                .into());
            }
            if let Some(command) = command {
                if cli_support::is_help_arg(&command) {
                    cli_support::print_usage();
                    return Ok(());
                }
                if cli_support::print_command_help(&command) {
                    return Ok(());
                }
                return Err(cli_support::unknown_command_error(&command).into());
            }
            cli_support::print_usage();
            Ok(())
        }
        Some("--help") | Some("-h") => {
            cli_support::print_usage();
            Ok(())
        }
        Some("--version") | Some("-V") => {
            cli_support::print_version();
            Ok(())
        }
        Some("start") => dispatch_with_help("start", args.collect(), daemon_cli::run_start),
        Some("stop") => dispatch_with_help("stop", args.collect(), daemon_cli::run_stop),
        Some("connect") => {
            dispatch_with_help("connect", args.collect(), connection_cli::run_connect)
        }
        Some("disconnect") => {
            dispatch_with_help("disconnect", args.collect(), connection_cli::run_disconnect)
        }
        Some("deploy") => dispatch_with_help("deploy", args.collect(), connection_cli::run_deploy),
        Some("add") => dispatch_with_help("add", args.collect(), sync_entry_cli::run_add),
        Some("get") => dispatch_with_help("get", args.collect(), sync_entry_cli::run_get),
        Some("remove") => dispatch_with_help("remove", args.collect(), sync_entry_cli::run_remove),
        Some("rm") => dispatch_with_help("rm", args.collect(), sync_entry_cli::run_remove),
        Some("doctor") => {
            dispatch_no_args_with_help("doctor", args.collect(), diagnostics_cli::run_doctor)
        }
        Some("status") => {
            dispatch_no_args_with_help("status", args.collect(), diagnostics_cli::run_status)
        }
        Some("log") => dispatch_no_args_with_help("log", args.collect(), diagnostics_cli::run_log),
        Some("history") => {
            dispatch_with_help("history", args.collect(), diagnostics_cli::run_history)
        }
        Some("compact") => {
            dispatch_with_help("compact", args.collect(), diagnostics_cli::run_compact)
        }
        Some("restore") => {
            dispatch_with_help("restore", args.collect(), diagnostics_cli::run_restore)
        }
        Some("redact") => dispatch_with_help("redact", args.collect(), diagnostics_cli::run_redact),
        Some("purge") => dispatch_with_help("purge", args.collect(), diagnostics_cli::run_purge),
        Some("daemon") => daemon_cli::run_daemon(args.collect()),
        Some(command) => Err(cli_support::unknown_command_error(command).into()),
        None => {
            cli_support::print_usage();
            Ok(())
        }
    }
}

fn dispatch_with_help(
    command: &str,
    args: Vec<String>,
    run_command: fn(Vec<String>) -> Result<(), Box<dyn Error>>,
) -> Result<(), Box<dyn Error>> {
    if args.len() == 1 && cli_support::is_help_arg(&args[0]) {
        cli_support::print_command_help(command);
        return Ok(());
    }
    run_command(args)
}

fn dispatch_no_args_with_help(
    command: &str,
    args: Vec<String>,
    run_command: fn() -> Result<(), Box<dyn Error>>,
) -> Result<(), Box<dyn Error>> {
    if args.len() == 1 && cli_support::is_help_arg(&args[0]) {
        cli_support::print_command_help(command);
        return Ok(());
    }
    if !args.is_empty() {
        return Err(format!("{command} does not accept arguments").into());
    }
    run_command()
}
