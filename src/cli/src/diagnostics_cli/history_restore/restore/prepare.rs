/**
@module PROJECTOR.EDGE.RESTORE_PREPARATION
Owns restore-time repo/profile/transport loading and resolution of the requested path into a concrete remote document and revision set.
*/
// @fileimplements PROJECTOR.EDGE.RESTORE_PREPARATION
use std::error::Error;
use std::fs;
use std::path::PathBuf;

use projector_domain::{
    CheckoutBinding, DocumentBodyRevision, DocumentId, ManifestEntry, RepoSyncConfig,
};
use projector_runtime::{FileServerProfileStore, HttpTransport, ProjectorHome, Transport};

use crate::cli_support::{normalize_projection_relative_path, repo_root};
use crate::sync_entry_cli::{
    load_sync_config, resolve_sync_target_for_requested_path, workspace_binding_for_target,
};

use super::super::args::RestoreArgs;

pub(super) struct PreparedRestore {
    pub(super) repo_root: PathBuf,
    pub(super) sync_config: RepoSyncConfig,
    pub(super) profiles: FileServerProfileStore,
    pub(super) restore_args: RestoreArgs,
    pub(super) requested_path: PathBuf,
    pub(super) binding: CheckoutBinding,
    pub(super) transport: HttpTransport,
    pub(super) document_id: DocumentId,
    pub(super) current_text: String,
    pub(super) all_revisions: Vec<DocumentBodyRevision>,
    pub(super) apply_target_override: Option<(PathBuf, PathBuf)>,
}

pub(super) fn prepare_restore(
    restore_args: RestoreArgs,
) -> Result<PreparedRestore, Box<dyn Error>> {
    let repo_root = repo_root()?;
    let sync_config = load_sync_config(&repo_root)?;
    let projector_home = ProjectorHome::discover()?;
    let profiles = FileServerProfileStore::new(projector_home);
    let sync_targets =
        projector_runtime::derive_sync_targets(&repo_root, &sync_config, Some(&profiles))?;
    let requested_path = normalize_projection_relative_path(&restore_args.repo_relative_path)?;
    let (target, mount_relative_path, relative_path) =
        resolve_sync_target_for_requested_path(&requested_path, &sync_targets)?;
    let binding = workspace_binding_for_target(target, &sync_targets)?;
    let server_addr = binding
        .server_addr
        .as_deref()
        .ok_or("restore requires a server-bound sync entry")?;
    let mut transport = HttpTransport::new(format!("http://{server_addr}"));
    let (snapshot, _) = transport.bootstrap(&binding)?;
    let current_entry = resolve_current_entry_for_repo_relative_path(&snapshot, &requested_path);
    let document_id = match &current_entry {
        Ok(entry) => entry.document_id.clone(),
        Err(_) => transport.resolve_document_by_historical_path(
            &binding,
            &mount_relative_path,
            &relative_path,
        )?,
    };
    let all_revisions = transport.list_body_revisions(&binding, &document_id, 10_000)?;
    let current_text = fs::read_to_string(repo_root.join(&requested_path)).unwrap_or_default();
    let apply_target_override = current_entry
        .err()
        .map(|_| (mount_relative_path.clone(), relative_path.clone()));

    Ok(PreparedRestore {
        repo_root,
        sync_config,
        profiles,
        restore_args,
        requested_path,
        binding,
        transport,
        document_id,
        current_text,
        all_revisions,
        apply_target_override,
    })
}

fn resolve_current_entry_for_repo_relative_path<'a>(
    snapshot: &'a projector_domain::BootstrapSnapshot,
    requested_path: &std::path::Path,
) -> Result<&'a ManifestEntry, Box<dyn Error>> {
    snapshot
        .manifest
        .entries
        .iter()
        .find(|entry| entry.mount_relative_path.join(&entry.relative_path) == requested_path)
        .ok_or_else(|| {
            format!(
                "no bound document found at current or deleted path {}",
                requested_path.display()
            )
            .into()
        })
}
