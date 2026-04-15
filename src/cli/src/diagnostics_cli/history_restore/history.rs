/**
@module PROJECTOR.EDGE.HISTORY_CLI
Owns document-history and workspace-history reads, including resolving the live requested document path against the authoritative server snapshot.
*/
// @fileimplements PROJECTOR.EDGE.HISTORY_CLI
use std::error::Error;
use std::path::Path;

use projector_domain::{BootstrapSnapshot, ManifestEntry};
use projector_runtime::{HttpTransport, Transport};

use crate::cli_support::{normalize_projection_relative_path, repo_root};
use crate::sync_entry_cli::{
    load_sync_targets_with_profiles, resolve_document_id_for_requested_path,
    resolve_sync_target_for_requested_path, single_workspace_binding, workspace_binding_for_target,
};

use super::args::{HistoryMode, parse_history_args};
use super::render::{print_body_revision, print_path_revision, print_workspace_snapshot};

pub(crate) fn run_history(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    let history_args = parse_history_args(&args)?;
    let repo_root = repo_root()?;
    let sync_targets = load_sync_targets_with_profiles(&repo_root)?;
    match history_args.mode {
        HistoryMode::DocumentPath {
            repo_relative_path,
            limit,
        } => {
            let requested_path = normalize_projection_relative_path(&repo_relative_path)?;
            let (target, mount_relative_path, relative_path) =
                resolve_sync_target_for_requested_path(&requested_path, &sync_targets)?;
            let binding = workspace_binding_for_target(target, &sync_targets)?;
            let server_addr = binding
                .server_addr
                .as_deref()
                .ok_or("history requires a server-bound sync entry")?;
            let mut transport = HttpTransport::new(format!("http://{server_addr}"));
            let (snapshot, _) = transport.bootstrap(&binding)?;
            let document_id = resolve_document_id_for_requested_path(
                &mut transport,
                &binding,
                &snapshot,
                &requested_path,
                &mount_relative_path,
                &relative_path,
            )?;

            let body_revisions = transport.list_body_revisions(&binding, &document_id, limit)?;
            let path_revisions = transport.list_path_revisions(&binding, &document_id, limit)?;

            println!("path: {}", requested_path.display());
            println!("document_id: {}", document_id.as_str());
            println!("body_revisions: {}", body_revisions.len());
            for revision in &body_revisions {
                print_body_revision(revision);
            }
            println!("path_revisions: {}", path_revisions.len());
            for revision in &path_revisions {
                print_path_revision(revision);
            }
        }
        HistoryMode::WorkspaceCursor { cursor } => {
            let binding = single_workspace_binding(&sync_targets)?;
            let server_addr = binding
                .server_addr
                .as_deref()
                .ok_or("workspace history requires server-bound sync entries")?;
            let mut transport = HttpTransport::new(format!("http://{server_addr}"));
            let snapshot = transport.reconstruct_workspace_at_cursor(&binding, cursor)?;
            print_workspace_snapshot(cursor, &snapshot);
        }
    }
    Ok(())
}

pub(crate) fn resolve_live_entry_for_repo_relative_path<'a>(
    snapshot: &'a BootstrapSnapshot,
    requested_path: &Path,
) -> Result<&'a ManifestEntry, Box<dyn Error>> {
    snapshot
        .manifest
        .entries
        .iter()
        .find(|entry| {
            !entry.deleted && entry.mount_relative_path.join(&entry.relative_path) == requested_path
        })
        .ok_or_else(|| {
            format!(
                "no live bound document found at {}",
                requested_path.display()
            )
            .into()
        })
}
