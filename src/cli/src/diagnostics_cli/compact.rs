/**
@module PROJECTOR.EDGE.COMPACT_CLI
Owns repo-local retained-history compaction policy reporting and override management for one file or folder path.
*/
// @fileimplements PROJECTOR.EDGE.COMPACT_CLI
use std::error::Error;
use std::path::PathBuf;

use projector_domain::HistoryCompactionPolicy;
use projector_runtime::{
    FileRepoSyncConfigStore, HistoryCompactionPolicySource, ResolvedHistoryCompactionPolicy,
};

use crate::cli_support::{normalize_projection_relative_path, repo_root};

struct CompactArgs {
    repo_relative_path: PathBuf,
    revisions: Option<usize>,
    frequency: Option<usize>,
    clear: bool,
}

pub(crate) fn run_compact(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    let compact_args = parse_compact_args(&args)?;
    let repo_root = repo_root()?;
    let store = FileRepoSyncConfigStore::new(&repo_root);

    if compact_args.clear {
        let removed = store.remove_history_compaction_policy(&compact_args.repo_relative_path)?;
        println!(
            "compact_policy: {}",
            if removed { "cleared" } else { "no_override" }
        );
        print_effective_policy(
            &compact_args.repo_relative_path,
            &store.resolve_history_compaction_policy(&compact_args.repo_relative_path)?,
        );
        return Ok(());
    }

    if let (Some(revisions), Some(frequency)) = (compact_args.revisions, compact_args.frequency) {
        store.upsert_history_compaction_policy(
            &compact_args.repo_relative_path,
            HistoryCompactionPolicy {
                revisions,
                frequency,
            },
        )?;
        println!("compact_policy: saved");
        print_effective_policy(
            &compact_args.repo_relative_path,
            &store.resolve_history_compaction_policy(&compact_args.repo_relative_path)?,
        );
        return Ok(());
    }

    print_effective_policy(
        &compact_args.repo_relative_path,
        &store.resolve_history_compaction_policy(&compact_args.repo_relative_path)?,
    );
    Ok(())
}

fn parse_compact_args(args: &[String]) -> Result<CompactArgs, Box<dyn Error>> {
    let mut revisions = None;
    let mut frequency = None;
    let mut clear = false;
    let mut repo_relative_path = None;
    let mut idx = 0;
    while idx < args.len() {
        match args[idx].as_str() {
            "--revisions" => {
                idx += 1;
                revisions = Some(
                    args.get(idx)
                        .ok_or("missing value after --revisions")?
                        .parse::<usize>()?,
                );
            }
            "--frequency" => {
                idx += 1;
                frequency = Some(
                    args.get(idx)
                        .ok_or("missing value after --frequency")?
                        .parse::<usize>()?,
                );
            }
            "--clear" => clear = true,
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
    if clear && (revisions.is_some() || frequency.is_some()) {
        return Err("compact does not accept --clear with --revisions or --frequency".into());
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
        clear,
    })
}

fn print_effective_policy(
    repo_relative_path: &std::path::Path,
    resolved: &ResolvedHistoryCompactionPolicy,
) {
    println!("path: {}", repo_relative_path.display());
    println!(
        "effective_policy: revisions={} frequency={}",
        resolved.policy.revisions, resolved.policy.frequency
    );
    match &resolved.source {
        HistoryCompactionPolicySource::Default => {
            println!("policy_source: default");
        }
        HistoryCompactionPolicySource::PathOverride { repo_relative_path } => {
            println!("policy_source: path_override");
            println!("source_path: {}", repo_relative_path.display());
        }
        HistoryCompactionPolicySource::AncestorOverride { repo_relative_path } => {
            println!("policy_source: ancestor_override");
            println!("source_path: {}", repo_relative_path.display());
        }
    }
}
