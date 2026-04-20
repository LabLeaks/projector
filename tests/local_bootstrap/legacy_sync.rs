/**
@module PROJECTOR.TESTS.SUPPORT.LOCAL_BOOTSTRAP_LEGACY_SYNC
Legacy `projector sync` compatibility harness used by local-bootstrap proofs while the newer explicit sync subcommands coexist.
*/
// @fileimplements PROJECTOR.TESTS.SUPPORT.LOCAL_BOOTSTRAP_LEGACY_SYNC
use super::*;

#[derive(Clone, Debug)]
struct LegacySyncArgs {
    server_addr: Option<String>,
    watch: bool,
    poll_ms: u64,
    watch_cycles: Option<usize>,
    mount_args: Vec<String>,
}

pub(crate) fn run_legacy_sync_with_env(
    repo_root: &Path,
    args: &[&str],
    envs: &[(&str, &str)],
) -> Result<String, String> {
    let parsed = parse_legacy_sync_args(args)?;
    let sync_store = FileRepoSyncConfigStore::new(repo_root);
    let mut sync_config = sync_store.load().map_err(|err| err.to_string())?;
    let created_binding = if sync_config.entries.is_empty() {
        if parsed.mount_args.is_empty() {
            return Err(
                "at least one projection path is required for first sync, for example: projector sync private"
                    .to_owned(),
            );
        }
        let projection_relative_paths =
            normalize_projection_relative_paths_for_test(&parsed.mount_args)?;
        for projection_relative_path in &projection_relative_paths {
            ensure_gitignored_for_test(repo_root, projection_relative_path)?;
        }
        let workspace_id = WorkspaceId::new(format!(
            "ws-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        let actor_id = ActorId::new(format!(
            "actor-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        let server_profile_id = parsed
            .server_addr
            .clone()
            .ok_or_else(|| "sync requires --server on first run".to_owned())?;
        sync_config = RepoSyncConfig {
            entries: projection_relative_paths
                .iter()
                .map(|path| RepoSyncEntry {
                    entry_id: sync_entry_id_for_test(path),
                    workspace_id: workspace_id.clone(),
                    actor_id: actor_id.clone(),
                    server_profile_id: server_profile_id.clone(),
                    local_relative_path: path.clone(),
                    remote_relative_path: path.clone(),
                    kind: SyncEntryKind::Directory,
                })
                .collect(),
        };
        sync_store
            .save(&sync_config)
            .map_err(|err| err.to_string())?;
        let home = projector_home_for_test(repo_root, envs);
        let registry = FileMachineSyncRegistryStore::new(home);
        let _ = registry
            .sync_repo(repo_root, &sync_config)
            .map_err(|err| err.to_string())?;
        true
    } else {
        if let Some(requested_server_addr) = parsed.server_addr.as_deref() {
            let server_addrs = sync_config
                .entries
                .iter()
                .map(|entry| entry.server_profile_id.as_str())
                .collect::<std::collections::BTreeSet<_>>();
            if server_addrs.len() != 1
                || server_addrs.iter().next().copied() != Some(requested_server_addr)
            {
                return Err(format!(
                    "checkout already bound to server {}; requested {}",
                    server_addrs.iter().copied().next().unwrap_or("none"),
                    requested_server_addr
                ));
            }
        }
        if !parsed.mount_args.is_empty() {
            let requested = normalize_projection_relative_paths_for_test(&parsed.mount_args)?;
            let existing = sync_config
                .entries
                .iter()
                .map(|entry| entry.local_relative_path.clone())
                .collect::<Vec<_>>();
            if requested != existing {
                return Err(format!(
                    "checkout already bound to {}; requested {}",
                    display_paths_for_test(&existing),
                    display_paths_for_test(&requested)
                ));
            }
        }
        false
    };

    let binding = load_workspace_binding_from_sync_config(repo_root);
    let local_event = StoredEvent {
        timestamp_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before unix epoch")
            .as_millis(),
        actor_id: binding.actor_id.clone(),
        kind: if created_binding {
            ProvenanceEventKind::SyncBootstrapped
        } else {
            ProvenanceEventKind::SyncReusedBinding
        },
        path: display_paths_for_test(&binding.projection_relative_paths),
        summary: if created_binding {
            "bootstrapped local projector state".to_owned()
        } else {
            "reused existing checkout binding".to_owned()
        },
    };
    FileProvenanceLog::new(repo_root.join(".projector/events.log"))
        .append(&local_event)
        .map_err(|err| err.to_string())?;

    let mut runner = SyncRunner::new(
        &binding,
        binding
            .server_addr
            .as_ref()
            .map(|server_addr| HttpTransport::new(format!("http://{server_addr}"))),
    );
    runner
        .run(&SyncLoopOptions {
            watch: parsed.watch,
            poll_ms: parsed.poll_ms,
            watch_cycles: parsed.watch_cycles,
        })
        .map_err(|err| err.to_string())?;

    let mut output = String::new();
    output.push_str(&format!("repo_root: {}\n", repo_root.display()));
    output.push_str(&format!(
        "workspace_id: {}\n",
        binding.workspace_id.as_str()
    ));
    output.push_str(&format!("actor_id: {}\n", binding.actor_id.as_str()));
    output.push_str(&format!(
        "projector_dir: {}\n",
        binding.roots.projector_dir.display()
    ));
    output.push_str(&format!(
        "server_addr: {}\n",
        binding.server_addr.as_deref().unwrap_or("none")
    ));
    output.push_str(&format!(
        "projection_paths: {}\n",
        display_paths_for_test(&binding.projection_relative_paths)
    ));
    for projection_dir in &binding.roots.projection_paths {
        output.push_str(&format!("projection_dir: {}\n", projection_dir.display()));
    }
    output.push_str(&format!(
        "binding: {}\n",
        if created_binding { "created" } else { "reused" }
    ));
    Ok(output)
}

fn parse_legacy_sync_args(args: &[&str]) -> Result<LegacySyncArgs, String> {
    let mut server_addr = None;
    let mut watch = false;
    let mut poll_ms = 250_u64;
    let mut watch_cycles = None;
    let mut mount_args = Vec::new();
    let mut idx = 1;
    while idx < args.len() {
        match args[idx] {
            "--server" => {
                idx += 1;
                server_addr = Some(
                    args.get(idx)
                        .ok_or_else(|| "missing address after --server".to_owned())?
                        .to_string(),
                );
            }
            "--watch" => watch = true,
            "--poll-ms" => {
                idx += 1;
                poll_ms = args
                    .get(idx)
                    .ok_or_else(|| "missing value after --poll-ms".to_owned())?
                    .parse::<u64>()
                    .map_err(|err| format!("invalid --poll-ms: {err}"))?;
            }
            "--watch-cycles" => {
                idx += 1;
                watch_cycles = Some(
                    args.get(idx)
                        .ok_or_else(|| "missing value after --watch-cycles".to_owned())?
                        .parse::<usize>()
                        .map_err(|err| format!("invalid --watch-cycles: {err}"))?,
                );
            }
            other => mount_args.push(other.to_owned()),
        }
        idx += 1;
    }

    Ok(LegacySyncArgs {
        server_addr,
        watch,
        poll_ms,
        watch_cycles,
        mount_args,
    })
}

fn normalize_projection_relative_paths_for_test(
    raw_paths: &[String],
) -> Result<Vec<PathBuf>, String> {
    let mut normalized = Vec::new();
    for raw in raw_paths {
        let path = normalize_projection_relative_path_for_test(raw)?;
        if !normalized.contains(&path) {
            normalized.push(path);
        }
    }

    for (idx, left) in normalized.iter().enumerate() {
        for right in normalized.iter().skip(idx + 1) {
            if left.starts_with(right) || right.starts_with(left) {
                return Err(format!(
                    "projection paths must not overlap: {} and {}",
                    left.display(),
                    right.display()
                ));
            }
        }
    }

    Ok(normalized)
}

fn normalize_projection_relative_path_for_test(raw: &str) -> Result<PathBuf, String> {
    let path = Path::new(raw);
    if path.as_os_str().is_empty() {
        return Err("projection path must not be empty".to_owned());
    }
    if path.is_absolute() {
        return Err("projection path must be repo-relative, not absolute".to_owned());
    }

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Normal(part) => normalized.push(part),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                return Err("projection path must not escape the repo root".to_owned());
            }
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                return Err("projection path must be repo-relative".to_owned());
            }
        }
    }

    if normalized.as_os_str().is_empty() {
        return Err("projection path must contain at least one normal path segment".to_owned());
    }
    if normalized == Path::new(".projector") || normalized.starts_with(".projector") {
        return Err("projection path must not live inside .projector".to_owned());
    }

    Ok(normalized)
}

fn display_paths_for_test(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(",")
}

fn sync_entry_id_for_test(path: &Path) -> String {
    let mut id = String::from("entry");
    for component in path.components() {
        if let std::path::Component::Normal(part) = component {
            id.push('-');
            id.push_str(&part.to_string_lossy());
        }
    }
    id
}

fn is_path_gitignored_for_test(
    repo_root: &Path,
    projection_relative_path: &Path,
) -> Result<bool, String> {
    let candidates = [
        projection_relative_path.display().to_string(),
        format!("{}/", projection_relative_path.display()),
    ];

    for candidate in candidates {
        let status = Command::new("git")
            .arg("check-ignore")
            .arg("-q")
            .arg(&candidate)
            .current_dir(repo_root)
            .status()
            .map_err(|err| err.to_string())?;

        if status.success() {
            return Ok(true);
        }
    }

    Ok(false)
}

fn ensure_gitignored_for_test(
    repo_root: &Path,
    projection_relative_path: &Path,
) -> Result<(), String> {
    if is_path_gitignored_for_test(repo_root, projection_relative_path)? {
        return Ok(());
    }
    Err(format!(
        "projection path {} is not gitignored",
        projection_relative_path.display()
    ))
}

fn projector_home_for_test(repo_root: &Path, envs: &[(&str, &str)]) -> ProjectorHome {
    let merged = merged_test_envs(repo_root, envs);
    let home = merged
        .iter()
        .find(|(key, _)| key == "PROJECTOR_HOME")
        .map(|(_, value)| value.clone())
        .expect("projector home env");
    ProjectorHome::new(home)
}
