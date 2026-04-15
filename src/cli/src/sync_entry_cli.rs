/**
@module PROJECTOR.EDGE.SYNC_ENTRY_CLI
Owns local-first `add`, remote-first `get`, and `remove` flows for whole sync entries, including profile resolution, bootstrap, and materialization.
*/
// @fileimplements PROJECTOR.EDGE.SYNC_ENTRY_CLI
use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use projector_domain::{
    ActorId, CheckoutBinding, DocumentId, ProjectionRoots, RepoSyncConfig, RepoSyncEntry,
    SyncContext, SyncEntryKind, SyncEntrySummary, SyncEntryTarget, WorkspaceId,
};
use projector_runtime::{
    FileProvenanceLog, FileRepoSyncConfigStore, FileServerProfileStore, HttpTransport,
    ProjectorHome, StoredEvent, SyncLoopOptions, SyncRunner, Transport,
    apply_authoritative_snapshot, derive_sync_targets,
};

use crate::cli_support::{
    ensure_gitignored, format_sync_entry_kind, infer_sync_entry_kind, is_path_tracked_by_git,
    make_id, normalize_projection_relative_path, repo_root, sync_entry_id,
    sync_machine_repo_registration,
};
use crate::connection_cli::resolve_profile_for_action;
use crate::get_browser::{GetBrowserExit, browse_sync_entries};

pub(crate) fn run_add(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    let add_args = parse_add_args(&args)?;
    let repo_root = repo_root()?;
    let requested_path = normalize_projection_relative_path(&add_args.path)?;
    ensure_gitignored(&repo_root, &requested_path)?;
    if is_path_tracked_by_git(&repo_root, &requested_path)? && !add_args.force {
        return Err(format!(
            "path {} is already under version control; rerun with --force to add it to projector",
            requested_path.display()
        )
        .into());
    }

    let sync_store = FileRepoSyncConfigStore::new(&repo_root);
    let existing_config = load_sync_config(&repo_root)?;
    let projector_home = ProjectorHome::discover()?;
    let profiles = FileServerProfileStore::new(projector_home);
    let existing_entry = existing_config
        .entries
        .iter()
        .find(|entry| entry.local_relative_path == requested_path)
        .cloned();

    let kind = infer_sync_entry_kind(&repo_root, &requested_path, &add_args.path);
    let entry = if let Some(mut entry) = existing_entry {
        entry.kind = kind.clone();
        entry
    } else {
        let actor_id = existing_config
            .entries
            .first()
            .map(|entry| entry.actor_id.clone())
            .unwrap_or_else(|| ActorId::new(make_id("actor")));
        let server_profile_id = resolve_profile_for_action(
            &profiles,
            add_args.server_profile_id.as_deref(),
            "projector add",
        )?
        .profile_id;
        RepoSyncEntry {
            entry_id: sync_entry_id(&requested_path),
            workspace_id: WorkspaceId::new(make_id("ws")),
            actor_id,
            server_profile_id,
            local_relative_path: requested_path.clone(),
            remote_relative_path: requested_path.clone(),
            kind,
        }
    };

    let mut next_config = existing_config.clone();
    if let Some(existing) = next_config
        .entries
        .iter_mut()
        .find(|existing| existing.local_relative_path == entry.local_relative_path)
    {
        *existing = entry.clone();
    } else {
        next_config.entries.push(entry.clone());
    }
    next_config
        .entries
        .sort_by(|left, right| left.local_relative_path.cmp(&right.local_relative_path));

    bootstrap_local_sync_entry(&repo_root, &next_config, &entry, &profiles)?;

    sync_store.save(&next_config)?;
    sync_machine_repo_registration(&repo_root)?;
    FileProvenanceLog::new(repo_root.join(".projector/events.log")).append(&StoredEvent {
        timestamp_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before unix epoch")
            .as_millis(),
        actor_id: entry.actor_id.clone(),
        kind: projector_domain::ProvenanceEventKind::SyncBootstrapped,
        path: entry.local_relative_path.display().to_string(),
        summary: "bootstrapped local sync entry".to_owned(),
    })?;

    println!("sync_entry: added");
    println!("path: {}", entry.local_relative_path.display());
    println!("kind: {}", format_sync_entry_kind(&entry.kind));
    println!("server_profile: {}", entry.server_profile_id);
    println!("workspace_id: {}", entry.workspace_id.as_str());
    Ok(())
}

pub(crate) fn run_remove(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    let remove_args = parse_remove_args(&args)?;
    let repo_root = repo_root()?;
    let requested_path = normalize_projection_relative_path(&remove_args.path)?;
    let sync_store = FileRepoSyncConfigStore::new(&repo_root);
    let _ = load_sync_config(&repo_root)?;
    let removed = sync_store.remove_entry(&requested_path)?;
    if !removed {
        return Err(format!(
            "path {} is not configured for projector sync",
            requested_path.display()
        )
        .into());
    }
    sync_machine_repo_registration(&repo_root)?;

    println!("sync_entry: removed");
    println!("path: {}", requested_path.display());
    Ok(())
}

pub(crate) fn run_get(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    let get_args = parse_get_args(&args)?;
    let repo_root = repo_root()?;
    let projector_home = ProjectorHome::discover()?;
    let profiles = FileServerProfileStore::new(projector_home.clone());
    let profile = resolve_profile_for_action(
        &profiles,
        get_args.server_profile_id.as_deref(),
        "projector get",
    )?;
    let transport = HttpTransport::new(format!("http://{}", profile.server_addr));
    let entry = match get_args.sync_entry_id.as_deref() {
        Some(sync_entry_id) => find_remote_sync_entry(&transport, sync_entry_id)?,
        None => {
            if !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) {
                return Err("projector get without an id requires an interactive terminal".into());
            }
            let entries = transport.list_sync_entries(100)?;
            match browse_sync_entries(&entries)? {
                GetBrowserExit::Selected(entry) => entry,
                GetBrowserExit::Cancelled => {
                    println!("get: cancelled");
                    return Ok(());
                }
            }
        }
    };

    let local_relative_path = match get_args.local_path.as_deref() {
        Some(local_path) => normalize_projection_relative_path(local_path)?,
        None => normalize_projection_relative_path(&entry.remote_path)?,
    };
    ensure_gitignored(&repo_root, &local_relative_path)?;
    ensure_path_not_tracked_or_existing(&repo_root, &local_relative_path)?;

    let current_config = load_sync_config(&repo_root)?;
    ensure_sync_entry_not_already_attached(&current_config, &entry, &local_relative_path)?;
    let actor_id = current_repo_actor_id(&current_config);
    let entry_config = RepoSyncEntry {
        entry_id: entry.sync_entry_id.clone(),
        workspace_id: WorkspaceId::new(entry.workspace_id.clone()),
        actor_id,
        server_profile_id: profile.profile_id.clone(),
        local_relative_path: local_relative_path.clone(),
        remote_relative_path: PathBuf::from(&entry.remote_path),
        kind: entry.kind.clone(),
    };

    let mut next_config = current_config.clone();
    next_config.entries.push(entry_config.clone());
    next_config
        .entries
        .sort_by(|left, right| left.local_relative_path.cmp(&right.local_relative_path));

    materialize_sync_config_entries(&repo_root, &next_config, &profiles)?;

    let sync_store = FileRepoSyncConfigStore::new(&repo_root);
    sync_store.save(&next_config)?;
    sync_machine_repo_registration(&repo_root)?;

    println!("sync_entry: retrieved");
    println!("sync_entry_id: {}", entry.sync_entry_id);
    println!("server_profile: {}", profile.profile_id);
    println!("remote_path: {}", entry.remote_path);
    println!("local_path: {}", local_relative_path.display());
    println!("kind: {}", format_sync_entry_kind(&entry.kind));
    if let Some(source_repo_name) = entry.source_repo_name.as_deref() {
        println!("source_repo: {}", source_repo_name);
    }
    if let Some(preview) = entry.preview.as_deref() {
        println!("preview: {}", preview);
    }
    Ok(())
}

pub(crate) fn load_sync_config(repo_root: &Path) -> Result<RepoSyncConfig, Box<dyn Error>> {
    Ok(FileRepoSyncConfigStore::new(repo_root).load()?)
}

pub(crate) fn load_sync_targets_with_profiles(
    repo_root: &Path,
) -> Result<Vec<SyncEntryTarget>, Box<dyn Error>> {
    let sync_config = load_sync_config(repo_root)?;
    let projector_home = ProjectorHome::discover()?;
    let profiles = FileServerProfileStore::new(projector_home);
    derive_sync_targets(repo_root, &sync_config, Some(&profiles)).map_err(Box::<dyn Error>::from)
}

pub(crate) fn group_sync_targets_by_workspace(targets: &[SyncEntryTarget]) -> Vec<CheckoutBinding> {
    let mut grouped = BTreeMap::<(String, String, String), Vec<SyncEntryTarget>>::new();
    for target in targets {
        grouped
            .entry((
                target.workspace_id.as_str().to_owned(),
                target.actor_id.as_str().to_owned(),
                target.server_addr.clone().unwrap_or_default(),
            ))
            .or_default()
            .push(target.clone());
    }

    grouped
        .into_values()
        .map(|targets| synthetic_materialization_binding_from_targets(&targets))
        .collect()
}

pub(crate) fn single_workspace_binding(
    targets: &[SyncEntryTarget],
) -> Result<CheckoutBinding, Box<dyn Error>> {
    let grouped = group_sync_targets_by_workspace(targets);
    match grouped.as_slice() {
        [] => Err("no configured projector sync entries".into()),
        [binding] => Ok(binding.clone()),
        _ => Err(
            "workspace-wide history requires exactly one workspace in this repo; use path-specific history instead"
                .into(),
        ),
    }
}

pub(crate) fn workspace_binding_for_target(
    target: &SyncEntryTarget,
    targets: &[SyncEntryTarget],
) -> Result<CheckoutBinding, Box<dyn Error>> {
    group_sync_targets_by_workspace(targets)
        .into_iter()
        .find(|binding| binding.workspace_id == target.workspace_id)
        .ok_or_else(|| "could not resolve workspace binding for sync entry target".into())
}

pub(crate) fn resolve_sync_target_for_requested_path<'a>(
    requested_path: &Path,
    targets: &'a [SyncEntryTarget],
) -> Result<(&'a SyncEntryTarget, PathBuf, PathBuf), Box<dyn Error>> {
    targets
        .iter()
        .find_map(|target| {
            let repo_root = target.projector_dir.parent()?;
            let local_relative_root = target.mount.absolute_path.strip_prefix(repo_root).ok()?;
            let local_relative_path = requested_path;
            match target.mount.kind {
                SyncEntryKind::File => {
                    if requested_path == local_relative_root {
                        Some((target, target.mount.relative_path.clone(), PathBuf::new()))
                    } else {
                        None
                    }
                }
                SyncEntryKind::Directory => local_relative_path
                    .strip_prefix(local_relative_root)
                    .ok()
                    .map(|relative| {
                        (
                            target,
                            target.mount.relative_path.clone(),
                            relative.to_path_buf(),
                        )
                    }),
            }
        })
        .ok_or_else(|| {
            format!(
                "path {} is not under a configured projector sync entry",
                requested_path.display()
            )
            .into()
        })
}

pub(crate) fn resolve_document_id_for_requested_path<T>(
    transport: &mut T,
    binding: &dyn SyncContext,
    snapshot: &projector_domain::BootstrapSnapshot,
    requested_path: &Path,
    mount_relative_path: &Path,
    relative_path: &Path,
) -> Result<DocumentId, Box<dyn Error>>
where
    T: Transport<Error = std::io::Error>,
{
    if let Ok(entry) =
        crate::diagnostics_cli::resolve_live_entry_for_repo_relative_path(snapshot, requested_path)
    {
        return Ok(entry.document_id.clone());
    }
    transport
        .resolve_document_by_historical_path(binding, mount_relative_path, relative_path)
        .map_err(Box::<dyn Error>::from)
}

pub(crate) fn materialize_sync_config_entries(
    repo_root: &Path,
    config: &RepoSyncConfig,
    profiles: &FileServerProfileStore,
) -> Result<(), Box<dyn Error>> {
    let targets = derive_sync_targets(repo_root, config, Some(profiles))?;
    if targets.is_empty() {
        return Ok(());
    }

    let mut grouped_targets =
        BTreeMap::<(String, String), Vec<projector_domain::SyncEntryTarget>>::new();
    for target in targets.clone() {
        let server_addr = target
            .server_addr
            .clone()
            .ok_or("sync entry target is missing a resolved server address")?;
        grouped_targets
            .entry((target.workspace_id.as_str().to_owned(), server_addr))
            .or_default()
            .push(target);
    }

    let mut merged_snapshot = projector_domain::BootstrapSnapshot::default();
    for group in grouped_targets.into_values() {
        let binding = synthetic_materialization_binding_from_targets(&group);
        let server_addr = binding
            .server_addr
            .as_deref()
            .ok_or("sync entry target is missing a resolved server address")?;
        let mut transport = HttpTransport::new(format!("http://{server_addr}"));
        let (snapshot, _) = transport.bootstrap(&binding)?;
        merged_snapshot = merge_bootstrap_snapshots(merged_snapshot, snapshot);
    }

    let binding = synthetic_materialization_binding_from_targets(&targets);
    apply_authoritative_snapshot(&binding, &merged_snapshot)?;
    Ok(())
}

pub(crate) fn synthetic_materialization_binding_from_targets(
    targets: &[projector_domain::SyncEntryTarget],
) -> CheckoutBinding {
    CheckoutBinding {
        workspace_id: targets
            .first()
            .map(|target| target.workspace_id.clone())
            .unwrap_or_else(|| WorkspaceId::new("ws-materialize")),
        actor_id: targets
            .first()
            .map(|target| target.actor_id.clone())
            .unwrap_or_else(|| ActorId::new("actor-materialize")),
        projection_relative_paths: targets
            .iter()
            .map(|target| target.mount.relative_path.clone())
            .collect(),
        projection_kinds: targets
            .iter()
            .map(|target| target.mount.kind.clone())
            .collect(),
        server_addr: targets
            .first()
            .and_then(|target| target.server_addr.clone()),
        roots: ProjectionRoots {
            projector_dir: targets
                .first()
                .map(|target| target.projector_dir.clone())
                .unwrap_or_else(|| PathBuf::from(".projector")),
            projection_paths: targets
                .iter()
                .map(|target| target.mount.absolute_path.clone())
                .collect(),
        },
    }
}

fn find_remote_sync_entry(
    transport: &HttpTransport,
    sync_entry_id: &str,
) -> Result<SyncEntrySummary, Box<dyn Error>> {
    let entries = transport.list_sync_entries(100)?;
    entries
        .into_iter()
        .find(|entry| entry.sync_entry_id == sync_entry_id)
        .ok_or_else(|| format!("remote sync entry {sync_entry_id} was not found").into())
}

fn ensure_path_not_tracked_or_existing(
    repo_root: &Path,
    local_relative_path: &Path,
) -> Result<(), Box<dyn Error>> {
    if is_path_tracked_by_git(repo_root, local_relative_path)? {
        return Err(format!(
            "path {} is already under version control; choose a different local path",
            local_relative_path.display()
        )
        .into());
    }

    let absolute_path = repo_root.join(local_relative_path);
    if absolute_path.exists() {
        return Err(format!(
            "path {} already exists locally; choose a different local path",
            local_relative_path.display()
        )
        .into());
    }

    Ok(())
}

fn ensure_sync_entry_not_already_attached(
    config: &RepoSyncConfig,
    entry: &SyncEntrySummary,
    local_relative_path: &Path,
) -> Result<(), Box<dyn Error>> {
    if let Some(existing) = config
        .entries
        .iter()
        .find(|existing| existing.local_relative_path == local_relative_path)
    {
        return Err(format!(
            "path {} is already configured for projector sync via entry {}",
            local_relative_path.display(),
            existing.entry_id
        )
        .into());
    }

    if let Some(existing) = config
        .entries
        .iter()
        .find(|existing| existing.entry_id == entry.sync_entry_id)
    {
        return Err(format!(
            "remote sync entry {} is already attached at {}",
            entry.sync_entry_id,
            existing.local_relative_path.display()
        )
        .into());
    }

    Ok(())
}

fn current_repo_actor_id(config: &RepoSyncConfig) -> ActorId {
    config
        .entries
        .first()
        .map(|entry| entry.actor_id.clone())
        .unwrap_or_else(|| ActorId::new(make_id("actor")))
}

fn bootstrap_local_sync_entry(
    repo_root: &Path,
    config: &RepoSyncConfig,
    entry: &RepoSyncEntry,
    profiles: &FileServerProfileStore,
) -> Result<(), Box<dyn Error>> {
    let targets = derive_sync_targets(repo_root, config, Some(profiles))?;
    let target = targets
        .iter()
        .find(|target| {
            target.entry_id == entry.entry_id
                && target.mount.relative_path == entry.remote_relative_path
                && target.mount.absolute_path == repo_root.join(&entry.local_relative_path)
        })
        .ok_or("new sync entry target could not be derived")?;

    let mut runner = SyncRunner::connect(target);
    runner.run(&SyncLoopOptions {
        watch: false,
        poll_ms: 250,
        watch_cycles: None,
    })?;
    materialize_sync_config_entries(repo_root, config, profiles)?;
    Ok(())
}

fn merge_bootstrap_snapshots(
    current: projector_domain::BootstrapSnapshot,
    delta: projector_domain::BootstrapSnapshot,
) -> projector_domain::BootstrapSnapshot {
    let mut entries_by_id = current
        .manifest
        .entries
        .into_iter()
        .map(|entry| (entry.document_id.clone(), entry))
        .collect::<HashMap<_, _>>();
    for entry in delta.manifest.entries {
        entries_by_id.insert(entry.document_id.clone(), entry);
    }

    let mut bodies_by_id = current
        .bodies
        .into_iter()
        .map(|body| (body.document_id.clone(), body))
        .collect::<HashMap<_, _>>();
    for body in delta.bodies {
        bodies_by_id.insert(body.document_id.clone(), body);
    }

    projector_domain::BootstrapSnapshot {
        manifest: projector_domain::ManifestState {
            entries: entries_by_id.into_values().collect(),
        },
        bodies: bodies_by_id.into_values().collect(),
    }
}

#[derive(Clone, Debug)]
struct AddArgs {
    path: String,
    force: bool,
    server_profile_id: Option<String>,
}

#[derive(Clone, Debug)]
struct RemoveArgs {
    path: String,
}

#[derive(Clone, Debug)]
struct GetArgs {
    server_profile_id: Option<String>,
    sync_entry_id: Option<String>,
    local_path: Option<String>,
}

fn parse_add_args(args: &[String]) -> Result<AddArgs, Box<dyn Error>> {
    let mut force = false;
    let mut server_profile_id = None;
    let mut path = None;
    let mut idx = 0;
    while idx < args.len() {
        match args[idx].as_str() {
            "--force" => {
                force = true;
            }
            "--profile" => {
                idx += 1;
                server_profile_id = Some(
                    args.get(idx)
                        .ok_or("missing value after --profile")?
                        .clone(),
                );
            }
            arg => {
                if path.is_none() {
                    path = Some(arg.to_owned());
                } else {
                    return Err(format!("unexpected extra add argument: {arg}").into());
                }
            }
        }
        idx += 1;
    }

    Ok(AddArgs {
        path: path.ok_or("add requires a repo-relative path argument")?,
        force,
        server_profile_id,
    })
}

fn parse_remove_args(args: &[String]) -> Result<RemoveArgs, Box<dyn Error>> {
    match args {
        [path] => Ok(RemoveArgs { path: path.clone() }),
        [] => Err("remove requires a repo-relative path argument".into()),
        _ => Err("remove accepts exactly one repo-relative path argument".into()),
    }
}

fn parse_get_args(args: &[String]) -> Result<GetArgs, Box<dyn Error>> {
    let mut server_profile_id = None;
    let mut positionals = Vec::new();
    let mut idx = 0;
    while idx < args.len() {
        match args[idx].as_str() {
            "--profile" => {
                idx += 1;
                server_profile_id = Some(
                    args.get(idx)
                        .ok_or("missing value after --profile")?
                        .clone(),
                );
            }
            arg => positionals.push(arg.to_owned()),
        }
        idx += 1;
    }

    match positionals.as_slice() {
        [] => Ok(GetArgs {
            server_profile_id,
            sync_entry_id: None,
            local_path: None,
        }),
        [sync_entry_id] => Ok(GetArgs {
            server_profile_id,
            sync_entry_id: Some(sync_entry_id.clone()),
            local_path: None,
        }),
        [sync_entry_id, local_path] => Ok(GetArgs {
            server_profile_id,
            sync_entry_id: Some(sync_entry_id.clone()),
            local_path: Some(local_path.clone()),
        }),
        _ => Err(
            "get accepts at most a sync-entry id and one optional repo-relative local path".into(),
        ),
    }
}
