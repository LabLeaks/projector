/**
@module PROJECTOR.EDGE.COMPACT_CLI
Owns server-backed retained-history compaction policy reporting and override management for one file or folder path.
*/
// @fileimplements PROJECTOR.EDGE.COMPACT_CLI
use std::error::Error;
use std::path::PathBuf;

use projector_domain::HistoryCompactionPolicy;
use projector_runtime::{HttpTransport, Transport};

use crate::cli_support::{normalize_projection_relative_path, repo_root};
use crate::sync_entry_cli::{
    load_sync_targets_with_profiles, resolve_sync_target_for_requested_path,
    workspace_binding_for_target,
};

struct CompactArgs {
    repo_relative_path: PathBuf,
    revisions: Option<u32>,
    frequency: Option<u32>,
    inherit: bool,
}

pub(crate) fn run_compact(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    let compact_args = parse_compact_args(&args)?;
    let repo_root = repo_root()?;
    let sync_targets = load_sync_targets_with_profiles(&repo_root)?;
    let (target, _, _) =
        resolve_sync_target_for_requested_path(&compact_args.repo_relative_path, &sync_targets)?;
    let binding = workspace_binding_for_target(target, &sync_targets)?;
    let server_addr = binding
        .server_addr
        .as_deref()
        .ok_or("compact requires a server-bound sync entry")?;
    let mut transport = HttpTransport::new(format!("http://{server_addr}"));

    if compact_args.inherit {
        let removed = transport
            .clear_history_compaction_policy(&binding, &compact_args.repo_relative_path)?;
        println!(
            "compact_policy: {}",
            if removed {
                "inherited"
            } else {
                "already_inherited"
            }
        );
        print_effective_policy(&compact_args.repo_relative_path, &mut transport, &binding)?;
        return Ok(());
    }

    if let (Some(revisions), Some(frequency)) = (compact_args.revisions, compact_args.frequency) {
        transport.set_history_compaction_policy(
            &binding,
            &compact_args.repo_relative_path,
            &HistoryCompactionPolicy {
                revisions,
                frequency,
            },
        )?;
        println!("compact_policy: saved");
        print_effective_policy(&compact_args.repo_relative_path, &mut transport, &binding)?;
        return Ok(());
    }

    print_effective_policy(&compact_args.repo_relative_path, &mut transport, &binding)?;
    Ok(())
}

fn parse_compact_args(args: &[String]) -> Result<CompactArgs, Box<dyn Error>> {
    let mut revisions = None;
    let mut frequency = None;
    let mut inherit = false;
    let mut repo_relative_path = None;
    let mut idx = 0;
    while idx < args.len() {
        match args[idx].as_str() {
            "--revisions" => {
                idx += 1;
                revisions = Some(
                    args.get(idx)
                        .ok_or("missing value after --revisions")?
                        .parse::<u32>()?,
                );
            }
            "--frequency" => {
                idx += 1;
                frequency = Some(
                    args.get(idx)
                        .ok_or("missing value after --frequency")?
                        .parse::<u32>()?,
                );
            }
            "--inherit" => inherit = true,
            other => {
                if repo_relative_path.is_some() {
                    return Err(format!("unexpected extra compact argument: {other}").into());
                }
                repo_relative_path = Some(normalize_projection_relative_path(other)?);
            }
        }
        idx += 1;
    }

    let repo_relative_path =
        repo_relative_path.ok_or("compact requires a repo-relative path argument")?;
    if inherit && (revisions.is_some() || frequency.is_some()) {
        return Err("compact does not accept --inherit with --revisions or --frequency".into());
    }
    if revisions.is_some() ^ frequency.is_some() {
        return Err("compact requires both --revisions and --frequency together".into());
    }
    if revisions.is_some_and(|value| value == 0) {
        return Err("--revisions must be greater than zero".into());
    }
    if frequency.is_some_and(|value| value == 0) {
        return Err("--frequency must be greater than zero".into());
    }

    Ok(CompactArgs {
        repo_relative_path,
        revisions,
        frequency,
        inherit,
    })
}

fn print_effective_policy(
    repo_relative_path: &std::path::Path,
    transport: &mut HttpTransport,
    binding: &dyn projector_domain::SyncContext,
) -> Result<(), Box<dyn Error>> {
    let (policy, source_kind, source_path) =
        transport.get_history_compaction_policy(binding, repo_relative_path)?;
    println!("path: {}", repo_relative_path.display());
    println!(
        "effective_policy: revisions={} frequency={}",
        policy.revisions, policy.frequency
    );
    println!("policy_source: {source_kind}");
    if let Some(source_path) = source_path {
        println!("source_path: {source_path}");
    }
    Ok(())
}
