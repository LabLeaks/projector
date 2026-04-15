/**
@module PROJECTOR.EDGE.STATUS_CLI
Owns the day-to-day `projector status` surface over repo-local sync entries, active daemon state, recent recovery data, and conflicted projected text.
*/
// @fileimplements PROJECTOR.EDGE.STATUS_CLI
use std::error::Error;

use projector_runtime::{
    FileMachineDaemonStateStore, FileRuntimeLeaseStore, FileRuntimeStatusStore, ProjectorHome,
};

use crate::cli_support::{display_paths, format_sync_entry_kind, repo_root};
use crate::sync_entry_cli::{
    group_sync_targets_by_workspace, load_sync_config, load_sync_targets_with_profiles,
};

use super::conflicts::find_conflicted_text_paths;

pub(crate) fn run_status() -> Result<(), Box<dyn Error>> {
    let repo_root = repo_root()?;
    let sync_config = load_sync_config(&repo_root)?;
    let sync_targets = load_sync_targets_with_profiles(&repo_root)?;
    let lease_store = FileRuntimeLeaseStore::new(repo_root.join(".projector/runtime.lock"));
    let active_runtime = lease_store.load_active()?;
    let machine_daemon =
        FileMachineDaemonStateStore::new(ProjectorHome::discover()?).load_active()?;
    let status_store = FileRuntimeStatusStore::new(repo_root.join(".projector/status.txt"));
    let status = status_store.load()?;

    println!("repo_root: {}", repo_root.display());
    println!("sync_entry_count: {}", sync_config.entries.len());
    for entry in &sync_config.entries {
        println!(
            "sync_entry: path={} kind={} server_profile={} workspace_id={}",
            entry.local_relative_path.display(),
            format_sync_entry_kind(&entry.kind),
            entry.server_profile_id,
            entry.workspace_id.as_str()
        );
    }
    println!(
        "projector_dir_exists: {}",
        repo_root.join(".projector").exists()
    );
    if sync_targets.is_empty() {
        println!("workspace_contexts: none");
    } else {
        for context in group_sync_targets_by_workspace(&sync_targets) {
            println!("workspace_id: {}", context.workspace_id.as_str());
            println!("actor_id: {}", context.actor_id.as_str());
            println!(
                "server_addr: {}",
                context.server_addr.as_deref().unwrap_or("none")
            );
            println!(
                "projection_paths: {}",
                display_paths(&context.projection_relative_paths)
            );
        }
    }
    let active_daemon = active_runtime
        .as_ref()
        .map(|lease| (lease.pid, lease.started_at_ms))
        .or_else(|| {
            machine_daemon
                .as_ref()
                .map(|daemon| (daemon.pid, daemon.started_at_ms))
        });
    println!("daemon_running: {}", active_daemon.is_some());
    if let Some((daemon_pid, daemon_started_at_ms)) = active_daemon {
        println!("daemon_pid: {}", daemon_pid);
        println!("daemon_started_at_ms: {}", daemon_started_at_ms);
    }
    println!("pending_local_changes: {}", status.pending_local_changes);
    println!(
        "last_sync_timestamp_ms: {}",
        status
            .last_sync_timestamp_ms
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_owned())
    );
    if status.recovery_attempt_count > 0 {
        println!("recovery_attempt_count: {}", status.recovery_attempt_count);
        println!(
            "last_recovery_action: {}",
            status.last_recovery_action.as_deref().unwrap_or("unknown")
        );
    }
    let conflicted_paths = find_conflicted_text_paths(
        &repo_root,
        &sync_targets
            .iter()
            .map(|target| target.mount.clone())
            .collect::<Vec<_>>(),
    )?;
    if !conflicted_paths.is_empty() {
        println!("conflicted_text_documents: {}", conflicted_paths.len());
        for conflicted_path in conflicted_paths {
            println!("conflicted_text_path: {}", conflicted_path.display());
        }
    }
    if status.sync_issue_count > 0 {
        println!("sync_issue_count: {}", status.sync_issue_count);
        println!(
            "last_sync_issue_code: {}",
            status.last_sync_issue_code.as_deref().unwrap_or("unknown")
        );
        println!(
            "last_sync_issue_disposition: {}",
            status
                .last_sync_issue_disposition
                .as_ref()
                .map(|value| value.as_str())
                .unwrap_or("unknown")
        );
        println!(
            "last_sync_issue: {}",
            status.last_sync_issue.as_deref().unwrap_or("unknown")
        );
    }
    Ok(())
}
