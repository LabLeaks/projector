use std::fs;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use projector_domain::{
    ActorId, BootstrapSnapshot, CheckoutBinding, DocumentBody, DocumentBodyRevision, DocumentId,
    DocumentKind, DocumentPathRevision, ListBodyRevisionsRequest, ListBodyRevisionsResponse,
    ListEventsRequest, ListEventsResponse, ListPathRevisionsRequest, ListPathRevisionsResponse,
    ManifestEntry, ManifestState, ProjectionRoots, ProvenanceEvent, ProvenanceEventKind,
    PurgeDocumentBodyHistoryRequest, RedactDocumentBodyHistoryRequest,
    ReconstructWorkspaceRequest, ReconstructWorkspaceResponse, RepoSyncConfig, RepoSyncEntry,
    ResolveHistoricalPathRequest, ResolveHistoricalPathResponse, RestoreWorkspaceRequest,
    SyncContext, SyncEntryKind, WorkspaceId,
};
use projector_runtime::{
    BindingStore, FileBindingStore, FileMachineSyncRegistryStore, FileProvenanceLog,
    FileRepoSyncConfigStore, FileRuntimeStatusStore, FileServerProfileStore, HttpTransport,
    ProjectorHome, RuntimeStatus, StoredEvent, SyncIssueDisposition, SyncLoopOptions, SyncRunner,
    Transport, derive_sync_targets,
};

fn temp_repo(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time before unix epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("projector-{name}-{unique}"));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create temp repo root");
    let status = Command::new("git")
        .arg("init")
        .arg("-q")
        .current_dir(&root)
        .status()
        .expect("git init");
    assert!(status.success(), "git init failed");
    fs::create_dir_all(root.join(".jj")).expect("create fake jj repo");
    root
}

fn temp_projector_home(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time before unix epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("projector-home-{name}-{unique}"));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create temp projector home");
    root
}

fn run_projector(repo_root: &Path, args: &[&str]) -> String {
    run_projector_with_env(repo_root, args, &[])
}

fn run_projector_home(repo_root: &Path, projector_home: &Path, args: &[&str]) -> String {
    let projector_home_str = projector_home.to_str().expect("projector home utf8");
    run_projector_with_env(repo_root, args, &[("PROJECTOR_HOME", projector_home_str)])
}

fn run_projector_with_env(repo_root: &Path, args: &[&str], envs: &[(&str, &str)]) -> String {
    if is_legacy_sync_command(args) {
        return run_legacy_sync_with_env(repo_root, args, envs)
            .unwrap_or_else(|stderr| panic!("command failed: {stderr}"));
    }
    let merged_envs = merged_test_envs(repo_root, envs);
    let output = Command::new(env!("CARGO_BIN_EXE_projector"))
        .args(args)
        .current_dir(repo_root)
        .envs(merged_envs)
        .output()
        .expect("run projector");
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf8 stdout")
}

fn run_projector_failure_with_env(
    repo_root: &Path,
    args: &[&str],
    envs: &[(&str, &str)],
) -> String {
    if is_legacy_sync_command(args) {
        return run_legacy_sync_with_env(repo_root, args, envs)
            .expect_err("legacy sync unexpectedly succeeded");
    }
    let merged_envs = merged_test_envs(repo_root, envs);
    let output = Command::new(env!("CARGO_BIN_EXE_projector"))
        .args(args)
        .current_dir(repo_root)
        .envs(merged_envs)
        .output()
        .expect("run projector");
    assert!(
        !output.status.success(),
        "command unexpectedly succeeded: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    String::from_utf8(output.stderr).expect("utf8 stderr")
}

fn run_projector_tty(repo_root: &Path, args: &[&str], input: &str) -> String {
    run_projector_tty_with_env(repo_root, args, input, &[])
}

fn run_projector_tty_with_env(
    repo_root: &Path,
    args: &[&str],
    input: &str,
    envs: &[(&str, &str)],
) -> String {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 40,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("open pty");
    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_projector"));
    cmd.cwd(repo_root);
    cmd.env("TERM", "xterm-256color");
    for (key, value) in merged_test_envs(repo_root, envs) {
        cmd.env(key, value);
    }
    for arg in args {
        cmd.arg(arg);
    }
    let mut child = pair
        .slave
        .spawn_command(cmd)
        .expect("spawn projector in pty");
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().expect("clone pty reader");
    let mut writer = pair.master.take_writer().expect("take pty writer");
    let output = Arc::new(Mutex::new(Vec::new()));
    let output_reader = Arc::clone(&output);
    let reader_thread = thread::spawn(move || {
        let mut buffer = Vec::new();
        reader.read_to_end(&mut buffer).expect("read pty output");
        *output_reader.lock().expect("lock output buffer") = buffer;
    });
    thread::sleep(Duration::from_millis(1000));
    writer.write_all(input.as_bytes()).expect("write pty input");
    writer.flush().expect("flush pty input");
    drop(writer);

    let status = child.wait().expect("wait for projector");
    reader_thread.join().expect("join pty reader");
    let output = String::from_utf8(output.lock().expect("lock output buffer").clone())
        .expect("utf8 pty output");
    assert!(status.success(), "tty command failed: {output}");
    output
}

fn install_fake_ssh_tools(root: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let bin_dir = root.join("fake-bin");
    fs::create_dir_all(&bin_dir).expect("create fake bin dir");
    let ssh_log = root.join("ssh.log");
    let scp_log = root.join("scp.log");

    fs::write(
        bin_dir.join("ssh"),
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"{}\"\nexit 0\n",
            ssh_log.display()
        ),
    )
    .expect("write fake ssh");
    fs::write(
        bin_dir.join("scp"),
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"{}\"\nexit 0\n",
            scp_log.display()
        ),
    )
    .expect("write fake scp");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(bin_dir.join("ssh"), fs::Permissions::from_mode(0o755))
            .expect("chmod fake ssh");
        fs::set_permissions(bin_dir.join("scp"), fs::Permissions::from_mode(0o755))
            .expect("chmod fake scp");
    }

    (bin_dir, ssh_log, scp_log)
}

fn merged_test_envs(repo_root: &Path, envs: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut merged = vec![(
        "PROJECTOR_HOME".to_owned(),
        repo_root.join(".projector-test-home").display().to_string(),
    )];
    for (key, value) in envs {
        if *key == "PROJECTOR_HOME" {
            merged[0] = (key.to_string(), value.to_string());
        } else {
            merged.push((key.to_string(), value.to_string()));
        }
    }
    merged
}

fn is_legacy_sync_command(args: &[&str]) -> bool {
    args.first() == Some(&"sync") && !matches!(args.get(1), Some(&"start" | &"stop" | &"status"))
}

#[derive(Clone, Debug)]
struct LegacySyncArgs {
    server_addr: Option<String>,
    watch: bool,
    poll_ms: u64,
    watch_cycles: Option<usize>,
    mount_args: Vec<String>,
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

fn run_legacy_sync_with_env(
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

fn spawn_server(state_dir: &Path) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind server addr");
    let addr = listener.local_addr().expect("local addr");
    projector_server::spawn_background(listener, state_dir.to_path_buf());
    std::thread::sleep(std::time::Duration::from_millis(150));
    addr
}

fn connect_profile(repo_root: &Path, projector_home: &Path, profile_id: &str, addr: &str) {
    run_projector_home(
        repo_root,
        projector_home,
        &["connect", "--id", profile_id, "--server", addr],
    );
}

fn add_sync_entry(repo_root: &Path, projector_home: &Path, addr: &str, path: &str) {
    connect_profile(repo_root, projector_home, "homebox", addr);
    run_projector_home(repo_root, projector_home, &["add", path]);
}

fn seed_remote_sync_entry(
    state_dir: &Path,
    workspace_id: &str,
    mount_relative_path: &str,
    kind: SyncEntryKind,
    source_repo_name: &str,
    snapshot: &BootstrapSnapshot,
) {
    let workspace_dir = state_dir.join("workspaces").join(workspace_id);
    fs::create_dir_all(&workspace_dir).expect("create workspace dir");
    let entry_kind = match kind {
        SyncEntryKind::File => "file",
        SyncEntryKind::Directory => "directory",
    };
    fs::write(
        workspace_dir.join("metadata.txt"),
        format!(
            "workspace_id={workspace_id}\nprojection_relative_path={mount_relative_path}\nsource_repo_name={source_repo_name}\nentry_kind={entry_kind}\n"
        ),
    )
    .expect("write metadata");
    projector_server::write_workspace_snapshot(state_dir, workspace_id, snapshot)
        .expect("write workspace snapshot");
}

fn list_body_revisions(
    addr: &str,
    workspace_id: &str,
    document_id: &str,
    limit: usize,
) -> Vec<DocumentBodyRevision> {
    reqwest::blocking::Client::new()
        .post(format!("http://{addr}/history/body/list"))
        .json(&ListBodyRevisionsRequest {
            workspace_id: workspace_id.to_owned(),
            document_id: document_id.to_owned(),
            limit,
        })
        .send()
        .expect("send body history request")
        .error_for_status()
        .expect("body history response status")
        .json::<ListBodyRevisionsResponse>()
        .expect("decode body history response")
        .revisions
}

fn purge_body_history(addr: &str, workspace_id: &str, actor_id: &str, document_id: &str) {
    reqwest::blocking::Client::new()
        .post(format!("http://{addr}/history/body/purge"))
        .json(&PurgeDocumentBodyHistoryRequest {
            workspace_id: workspace_id.to_owned(),
            actor_id: actor_id.to_owned(),
            document_id: document_id.to_owned(),
        })
        .send()
        .expect("send body history purge request")
        .error_for_status()
        .expect("body history purge response status");
}

fn redact_body_history(
    addr: &str,
    workspace_id: &str,
    actor_id: &str,
    document_id: &str,
    exact_text: &str,
) {
    reqwest::blocking::Client::new()
        .post(format!("http://{addr}/history/body/redact"))
        .json(&RedactDocumentBodyHistoryRequest {
            workspace_id: workspace_id.to_owned(),
            actor_id: actor_id.to_owned(),
            document_id: document_id.to_owned(),
            exact_text: exact_text.to_owned(),
        })
        .send()
        .expect("send body history redact request")
        .error_for_status()
        .expect("body history redact response status");
}

fn list_events(addr: &str, workspace_id: &str, limit: usize) -> Vec<ProvenanceEvent> {
    reqwest::blocking::Client::new()
        .post(format!("http://{addr}/events/list"))
        .json(&ListEventsRequest {
            workspace_id: workspace_id.to_owned(),
            limit,
        })
        .send()
        .expect("send events request")
        .error_for_status()
        .expect("events response status")
        .json::<ListEventsResponse>()
        .expect("decode events response")
        .events
}

fn list_path_revisions(
    addr: &str,
    workspace_id: &str,
    document_id: &str,
    limit: usize,
) -> Vec<DocumentPathRevision> {
    reqwest::blocking::Client::new()
        .post(format!("http://{addr}/history/path/list"))
        .json(&ListPathRevisionsRequest {
            workspace_id: workspace_id.to_owned(),
            document_id: document_id.to_owned(),
            limit,
        })
        .send()
        .expect("send path history request")
        .error_for_status()
        .expect("path history response status")
        .json::<ListPathRevisionsResponse>()
        .expect("decode path history response")
        .revisions
}

fn resolve_document_by_historical_path(
    addr: &str,
    workspace_id: &str,
    mount_relative_path: &str,
    relative_path: &str,
) -> String {
    reqwest::blocking::Client::new()
        .post(format!("http://{addr}/history/path/resolve"))
        .json(&ResolveHistoricalPathRequest {
            workspace_id: workspace_id.to_owned(),
            mount_relative_path: mount_relative_path.to_owned(),
            relative_path: relative_path.to_owned(),
        })
        .send()
        .expect("send historical path resolve request")
        .error_for_status()
        .expect("historical path resolve response status")
        .json::<ResolveHistoricalPathResponse>()
        .expect("decode historical path resolve response")
        .document_id
}

fn reconstruct_workspace_at_cursor(
    addr: &str,
    workspace_id: &str,
    cursor: u64,
) -> BootstrapSnapshot {
    reqwest::blocking::Client::new()
        .post(format!("http://{addr}/history/workspace/reconstruct"))
        .json(&ReconstructWorkspaceRequest {
            workspace_id: workspace_id.to_owned(),
            cursor,
        })
        .send()
        .expect("send workspace reconstruction request")
        .error_for_status()
        .expect("workspace reconstruction response status")
        .json::<ReconstructWorkspaceResponse>()
        .expect("decode workspace reconstruction response")
        .snapshot
}

fn restore_workspace_at_cursor(
    addr: &str,
    workspace_id: &str,
    actor_id: &str,
    based_on_cursor: u64,
    cursor: u64,
) {
    reqwest::blocking::Client::new()
        .post(format!("http://{addr}/history/workspace/restore"))
        .json(&RestoreWorkspaceRequest {
            workspace_id: workspace_id.to_owned(),
            actor_id: actor_id.to_owned(),
            based_on_cursor: Some(based_on_cursor),
            cursor,
        })
        .send()
        .expect("send workspace restore request")
        .error_for_status()
        .expect("workspace restore response status");
}

fn load_workspace_binding_from_sync_config(repo_root: &Path) -> CheckoutBinding {
    let config = FileRepoSyncConfigStore::new(repo_root)
        .load()
        .expect("load sync config");
    let targets = derive_sync_targets(repo_root, &config, None).expect("derive sync targets");
    let first = targets.first().expect("configured sync targets");
    CheckoutBinding {
        workspace_id: first.workspace_id.clone(),
        actor_id: first.actor_id.clone(),
        server_addr: first.server_addr.clone(),
        roots: ProjectionRoots {
            projector_dir: repo_root.join(".projector"),
            projection_paths: targets
                .iter()
                .map(|target| target.mount.absolute_path.clone())
                .collect(),
        },
        projection_relative_paths: targets
            .iter()
            .map(|target| target.mount.relative_path.clone())
            .collect(),
        projection_kinds: targets
            .iter()
            .map(|target| target.mount.kind.clone())
            .collect(),
    }
}

fn clone_sync_config_for_repo(source_repo: &Path, dest_repo: &Path, actor_id: &str) {
    let config = FileRepoSyncConfigStore::new(source_repo)
        .load()
        .expect("load source sync config");
    let cloned = RepoSyncConfig {
        entries: config
            .entries
            .into_iter()
            .map(|mut entry| {
                entry.actor_id = ActorId::new(actor_id);
                entry
            })
            .collect(),
    };
    FileRepoSyncConfigStore::new(dest_repo)
        .save(&cloned)
        .expect("save cloned sync config");
}

fn save_sync_config_for_binding(repo_root: &Path, binding: &CheckoutBinding) {
    let config = RepoSyncConfig {
        entries: binding
            .projection_relative_paths
            .iter()
            .cloned()
            .zip(binding.projection_kinds.iter().cloned())
            .map(|(path, kind)| RepoSyncEntry {
                entry_id: format!("entry-{}", path.display()),
                workspace_id: binding.workspace_id.clone(),
                actor_id: binding.actor_id.clone(),
                server_profile_id: binding
                    .server_addr
                    .clone()
                    .unwrap_or_else(|| "none".to_owned()),
                local_relative_path: path.clone(),
                remote_relative_path: path,
                kind,
            })
            .collect(),
    };
    FileRepoSyncConfigStore::new(repo_root)
        .save(&config)
        .expect("save sync config for binding");
}

#[derive(Clone, Debug)]
struct RejectCreateTransport;

impl Transport for RejectCreateTransport {
    type Error = io::Error;

    fn bootstrap(
        &mut self,
        _binding: &dyn SyncContext,
    ) -> Result<(BootstrapSnapshot, u64), Self::Error> {
        Ok((BootstrapSnapshot::default(), 0))
    }

    fn changes_since(
        &mut self,
        _binding: &dyn SyncContext,
        _since_cursor: u64,
    ) -> Result<(BootstrapSnapshot, u64), Self::Error> {
        Ok((BootstrapSnapshot::default(), 0))
    }

    fn create_document(
        &mut self,
        _binding: &dyn SyncContext,
        _based_on_cursor: u64,
        _mount_relative_path: &Path,
        _relative_path: &Path,
        _text: &str,
    ) -> Result<DocumentId, Self::Error> {
        Err(io::Error::other(
            "create document request failed with status 409 Conflict: stale_cursor: manifest write based on stale cursor 0; current workspace cursor is 1",
        ))
    }

    fn update_document(
        &mut self,
        _binding: &dyn SyncContext,
        _document_id: &DocumentId,
        _base_text: &str,
        _text: &str,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn delete_document(
        &mut self,
        _binding: &dyn SyncContext,
        _based_on_cursor: u64,
        _document_id: &DocumentId,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn move_document(
        &mut self,
        _binding: &dyn SyncContext,
        _based_on_cursor: u64,
        _document_id: &DocumentId,
        _mount_relative_path: &Path,
        _relative_path: &Path,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn provenance(
        &mut self,
        _binding: &dyn SyncContext,
        _limit: usize,
    ) -> Result<Vec<ProvenanceEvent>, Self::Error> {
        Ok(Vec::new())
    }

    fn list_body_revisions(
        &mut self,
        _binding: &dyn SyncContext,
        _document_id: &DocumentId,
        _limit: usize,
    ) -> Result<Vec<DocumentBodyRevision>, Self::Error> {
        Ok(Vec::new())
    }

    fn list_path_revisions(
        &mut self,
        _binding: &dyn SyncContext,
        _document_id: &DocumentId,
        _limit: usize,
    ) -> Result<Vec<DocumentPathRevision>, Self::Error> {
        Ok(Vec::new())
    }

    fn reconstruct_workspace_at_cursor(
        &mut self,
        _binding: &dyn SyncContext,
        _cursor: u64,
    ) -> Result<BootstrapSnapshot, Self::Error> {
        Ok(BootstrapSnapshot::default())
    }

    fn restore_workspace_at_cursor(
        &mut self,
        _binding: &dyn SyncContext,
        _based_on_cursor: u64,
        _cursor: u64,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn restore_document_body_revision(
        &mut self,
        _binding: &dyn SyncContext,
        _document_id: &DocumentId,
        _seq: u64,
        _target_mount_relative_path: Option<&Path>,
        _target_relative_path: Option<&Path>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn redact_document_body_history(
        &mut self,
        _binding: &dyn SyncContext,
        _document_id: &DocumentId,
        _exact_text: &str,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn purge_document_body_history(
        &mut self,
        _binding: &dyn SyncContext,
        _document_id: &DocumentId,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn resolve_document_by_historical_path(
        &mut self,
        _binding: &dyn SyncContext,
        _mount_relative_path: &Path,
        _relative_path: &Path,
    ) -> Result<DocumentId, Self::Error> {
        Ok(DocumentId::new("doc-historical"))
    }
}

#[derive(Clone, Debug, Default)]
struct RetryAfterRebootstrapTransport {
    create_attempts: usize,
    created: Option<(DocumentId, String)>,
}

impl Transport for RetryAfterRebootstrapTransport {
    type Error = io::Error;

    fn bootstrap(
        &mut self,
        _binding: &dyn SyncContext,
    ) -> Result<(BootstrapSnapshot, u64), Self::Error> {
        let mut snapshot = BootstrapSnapshot::default();
        let mut cursor = 0;
        if let Some((document_id, text)) = &self.created {
            snapshot.manifest.entries.push(ManifestEntry {
                document_id: document_id.clone(),
                mount_relative_path: PathBuf::from("private"),
                relative_path: PathBuf::from("briefs/retry.html"),
                kind: DocumentKind::Text,
                deleted: false,
            });
            snapshot.bodies.push(DocumentBody {
                document_id: document_id.clone(),
                text: text.clone(),
            });
            cursor = 1;
        }
        Ok((snapshot, cursor))
    }

    fn changes_since(
        &mut self,
        _binding: &dyn SyncContext,
        since_cursor: u64,
    ) -> Result<(BootstrapSnapshot, u64), Self::Error> {
        if since_cursor >= 1 {
            return Ok((BootstrapSnapshot::default(), since_cursor));
        }
        self.bootstrap(_binding)
    }

    fn create_document(
        &mut self,
        _binding: &dyn SyncContext,
        _based_on_cursor: u64,
        _mount_relative_path: &Path,
        _relative_path: &Path,
        text: &str,
    ) -> Result<DocumentId, Self::Error> {
        self.create_attempts += 1;
        if self.create_attempts == 1 {
            return Err(io::Error::other(
                "create document request failed with status 409 Conflict: stale_cursor: manifest write based on stale cursor 0; current workspace cursor is 1",
            ));
        }

        let document_id = DocumentId::new("doc-retried");
        self.created = Some((document_id.clone(), text.to_owned()));
        Ok(document_id)
    }

    fn update_document(
        &mut self,
        _binding: &dyn SyncContext,
        _document_id: &DocumentId,
        _base_text: &str,
        _text: &str,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn delete_document(
        &mut self,
        _binding: &dyn SyncContext,
        _based_on_cursor: u64,
        _document_id: &DocumentId,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn move_document(
        &mut self,
        _binding: &dyn SyncContext,
        _based_on_cursor: u64,
        _document_id: &DocumentId,
        _mount_relative_path: &Path,
        _relative_path: &Path,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn provenance(
        &mut self,
        _binding: &dyn SyncContext,
        _limit: usize,
    ) -> Result<Vec<ProvenanceEvent>, Self::Error> {
        Ok(Vec::new())
    }

    fn list_body_revisions(
        &mut self,
        _binding: &dyn SyncContext,
        _document_id: &DocumentId,
        _limit: usize,
    ) -> Result<Vec<DocumentBodyRevision>, Self::Error> {
        Ok(Vec::new())
    }

    fn list_path_revisions(
        &mut self,
        _binding: &dyn SyncContext,
        _document_id: &DocumentId,
        _limit: usize,
    ) -> Result<Vec<DocumentPathRevision>, Self::Error> {
        Ok(Vec::new())
    }

    fn reconstruct_workspace_at_cursor(
        &mut self,
        _binding: &dyn SyncContext,
        _cursor: u64,
    ) -> Result<BootstrapSnapshot, Self::Error> {
        self.bootstrap(_binding).map(|(snapshot, _)| snapshot)
    }

    fn restore_workspace_at_cursor(
        &mut self,
        _binding: &dyn SyncContext,
        _based_on_cursor: u64,
        _cursor: u64,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn restore_document_body_revision(
        &mut self,
        _binding: &dyn SyncContext,
        _document_id: &DocumentId,
        _seq: u64,
        _target_mount_relative_path: Option<&Path>,
        _target_relative_path: Option<&Path>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn redact_document_body_history(
        &mut self,
        _binding: &dyn SyncContext,
        _document_id: &DocumentId,
        _exact_text: &str,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn purge_document_body_history(
        &mut self,
        _binding: &dyn SyncContext,
        _document_id: &DocumentId,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn resolve_document_by_historical_path(
        &mut self,
        _binding: &dyn SyncContext,
        _mount_relative_path: &Path,
        _relative_path: &Path,
    ) -> Result<DocumentId, Self::Error> {
        Ok(DocumentId::new("doc-historical"))
    }
}

#[derive(Clone, Debug, Default)]
struct RetryImmediatelyTransport {
    create_attempts: usize,
    created: Option<(DocumentId, String)>,
}

impl Transport for RetryImmediatelyTransport {
    type Error = io::Error;

    fn bootstrap(
        &mut self,
        _binding: &dyn SyncContext,
    ) -> Result<(BootstrapSnapshot, u64), Self::Error> {
        let mut snapshot = BootstrapSnapshot::default();
        let mut cursor = 0;
        if let Some((document_id, text)) = &self.created {
            snapshot.manifest.entries.push(ManifestEntry {
                document_id: document_id.clone(),
                mount_relative_path: PathBuf::from("private"),
                relative_path: PathBuf::from("briefs/transient.html"),
                kind: DocumentKind::Text,
                deleted: false,
            });
            snapshot.bodies.push(DocumentBody {
                document_id: document_id.clone(),
                text: text.clone(),
            });
            cursor = 1;
        }
        Ok((snapshot, cursor))
    }

    fn changes_since(
        &mut self,
        binding: &dyn SyncContext,
        since_cursor: u64,
    ) -> Result<(BootstrapSnapshot, u64), Self::Error> {
        if since_cursor >= 1 {
            return Ok((BootstrapSnapshot::default(), since_cursor));
        }
        self.bootstrap(binding)
    }

    fn create_document(
        &mut self,
        _binding: &dyn SyncContext,
        _based_on_cursor: u64,
        _mount_relative_path: &Path,
        _relative_path: &Path,
        text: &str,
    ) -> Result<DocumentId, Self::Error> {
        self.create_attempts += 1;
        if self.create_attempts == 1 {
            return Err(io::Error::other("tcp connect error: connection refused"));
        }

        let document_id = DocumentId::new("doc-transient");
        self.created = Some((document_id.clone(), text.to_owned()));
        Ok(document_id)
    }

    fn update_document(
        &mut self,
        _binding: &dyn SyncContext,
        _document_id: &DocumentId,
        _base_text: &str,
        _text: &str,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn delete_document(
        &mut self,
        _binding: &dyn SyncContext,
        _based_on_cursor: u64,
        _document_id: &DocumentId,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn move_document(
        &mut self,
        _binding: &dyn SyncContext,
        _based_on_cursor: u64,
        _document_id: &DocumentId,
        _mount_relative_path: &Path,
        _relative_path: &Path,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn provenance(
        &mut self,
        _binding: &dyn SyncContext,
        _limit: usize,
    ) -> Result<Vec<ProvenanceEvent>, Self::Error> {
        Ok(Vec::new())
    }

    fn list_body_revisions(
        &mut self,
        _binding: &dyn SyncContext,
        _document_id: &DocumentId,
        _limit: usize,
    ) -> Result<Vec<DocumentBodyRevision>, Self::Error> {
        Ok(Vec::new())
    }

    fn list_path_revisions(
        &mut self,
        _binding: &dyn SyncContext,
        _document_id: &DocumentId,
        _limit: usize,
    ) -> Result<Vec<DocumentPathRevision>, Self::Error> {
        Ok(Vec::new())
    }

    fn reconstruct_workspace_at_cursor(
        &mut self,
        _binding: &dyn SyncContext,
        _cursor: u64,
    ) -> Result<BootstrapSnapshot, Self::Error> {
        self.bootstrap(_binding).map(|(snapshot, _)| snapshot)
    }

    fn restore_workspace_at_cursor(
        &mut self,
        _binding: &dyn SyncContext,
        _based_on_cursor: u64,
        _cursor: u64,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn restore_document_body_revision(
        &mut self,
        _binding: &dyn SyncContext,
        _document_id: &DocumentId,
        _seq: u64,
        _target_mount_relative_path: Option<&Path>,
        _target_relative_path: Option<&Path>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn redact_document_body_history(
        &mut self,
        _binding: &dyn SyncContext,
        _document_id: &DocumentId,
        _exact_text: &str,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn purge_document_body_history(
        &mut self,
        _binding: &dyn SyncContext,
        _document_id: &DocumentId,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn resolve_document_by_historical_path(
        &mut self,
        _binding: &dyn SyncContext,
        _mount_relative_path: &Path,
        _relative_path: &Path,
    ) -> Result<DocumentId, Self::Error> {
        Ok(DocumentId::new("doc-historical"))
    }
}

// @verifies PROJECTOR.WORKSPACE.PROJECTION_ROOT
#[test]
fn sync_uses_configured_projection_mounts_instead_of_a_hardcoded_root() {
    let repo = temp_repo("projection-root");
    let projector_home = temp_projector_home("projection-root");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    connect_profile(&repo, &projector_home, "homebox", &addr);
    let stdout = run_projector_home(&repo, &projector_home, &["add", "private"]);

    assert!(stdout.contains("path: private"));
    assert!(repo.join("private").is_dir());
    assert!(!repo.join("_project").exists());
}

// @verifies PROJECTOR.BINDING.REPO_LOCAL_METADATA
#[test]
fn sync_keeps_binding_and_runtime_metadata_under_projector_dir() {
    let repo = temp_repo("repo-local-metadata");
    let projector_home = temp_projector_home("repo-local-metadata");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    add_sync_entry(&repo, &projector_home, &addr, "private");

    assert!(repo.join(".projector/sync-entries.json").is_file());
    assert!(repo.join(".projector/status.txt").is_file());
    assert!(repo.join(".projector/events.log").is_file());
    assert!(!repo.join("private/.projector").exists());
}

#[test]
fn connect_saves_and_reports_machine_global_server_profiles() {
    let repo = temp_repo("connect-profiles");
    let projector_home = temp_projector_home("connect-profiles");
    let projector_home_str = projector_home.to_str().expect("projector home utf8");

    let add_stdout = run_projector_with_env(
        &repo,
        &["connect", "--id", "homebox", "--server", "127.0.0.1:7000"],
        &[("PROJECTOR_HOME", projector_home_str)],
    );
    assert!(add_stdout.contains("connection: added"));
    assert!(add_stdout.contains("profile: homebox"));

    let list_stdout = run_projector_with_env(
        &repo,
        &["connect", "status"],
        &[("PROJECTOR_HOME", projector_home_str)],
    );
    assert!(list_stdout.contains("connection_count: 1"));
    assert!(list_stdout.contains("connection: id=homebox"));
    assert!(list_stdout.contains("server_addr=127.0.0.1:7000"));

    let registry = FileServerProfileStore::new(ProjectorHome::new(&projector_home))
        .load()
        .expect("load profile registry");
    assert_eq!(registry.profiles.len(), 1);
    assert_eq!(registry.profiles[0].server_addr, "127.0.0.1:7000");
}

// @verifies PROJECTOR.CLI.CONNECT.REPORTS_SERVER_STATUS
#[test]
fn connect_status_reports_reachability_for_all_profiles() {
    let repo = temp_repo("connect-status");
    let projector_home = temp_projector_home("connect-status");
    let projector_home_str = projector_home.to_str().expect("projector home utf8");
    fs::write(repo.join(".gitignore"), "private/\n").expect("write gitignore");
    fs::create_dir_all(repo.join("private")).expect("create private dir");
    let state_dir = projector_home.join("server-state");
    fs::create_dir_all(&state_dir).expect("create server state dir");
    let addr = spawn_server(&state_dir);

    run_projector_with_env(
        &repo,
        &["connect", "--id", "homebox", "--server", &addr.to_string()],
        &[("PROJECTOR_HOME", projector_home_str)],
    );
    run_projector_with_env(
        &repo,
        &["connect", "--id", "workbox", "--server", "127.0.0.1:9"],
        &[("PROJECTOR_HOME", projector_home_str)],
    );
    run_projector_with_env(
        &repo,
        &["add", "--profile", "homebox", "private"],
        &[("PROJECTOR_HOME", projector_home_str)],
    );

    let status_stdout = run_projector_with_env(
        &repo,
        &["connect", "status"],
        &[("PROJECTOR_HOME", projector_home_str)],
    );
    assert!(status_stdout.contains("connection_count: 2"));
    assert!(status_stdout.contains("connection: id=homebox"));
    assert!(status_stdout.contains(&format!("server_addr={addr}")));
    assert!(status_stdout.contains("reachable=true"));
    assert!(status_stdout.contains("repo_count=1"));
    assert!(status_stdout.contains("sync_entry_count=1"));
    assert!(status_stdout.contains("connection_sync_entry: id=homebox"));
    assert!(status_stdout.contains("path=private"));
    assert!(status_stdout.contains("kind=directory"));
    assert!(status_stdout.contains("connection: id=workbox"));
    assert!(status_stdout.contains("server_addr=127.0.0.1:9"));
    assert!(status_stdout.contains("reachable=false"));
    assert!(status_stdout.contains("repo_count=0"));
    assert!(status_stdout.contains("sync_entry_count=0"));
}

// @verifies PROJECTOR.CLI.DOCTOR.REPORTS_PROFILE_AND_REACHABILITY
#[test]
fn doctor_reports_clean_profile_reachability_and_sync_entry_sanity() {
    let repo = temp_repo("doctor-clean");
    let projector_home = temp_projector_home("doctor-clean");
    let projector_home_str = projector_home.to_str().expect("projector home utf8");
    fs::write(repo.join(".gitignore"), "private/\n").expect("write gitignore");
    fs::create_dir_all(repo.join("private")).expect("create private dir");

    let state_dir = projector_home.join("server-state");
    fs::create_dir_all(&state_dir).expect("create server state dir");
    let addr = spawn_server(&state_dir).to_string();

    run_projector_with_env(
        &repo,
        &["connect", "--id", "homebox", "--server", &addr],
        &[("PROJECTOR_HOME", projector_home_str)],
    );
    run_projector_with_env(
        &repo,
        &["add", "private"],
        &[("PROJECTOR_HOME", projector_home_str)],
    );

    let doctor = run_projector_with_env(
        &repo,
        &["doctor"],
        &[("PROJECTOR_HOME", projector_home_str)],
    );

    assert!(doctor.contains("connected_profile_count: 1"));
    assert!(doctor.contains("machine_daemon_running: false"));
    assert!(doctor.contains("repo_registered: true"));
    assert!(doctor.contains("runtime_lease_active: false"));
    assert!(doctor.contains("recent_sync_issue_count: 0"));
    assert!(doctor.contains("sync_entry_count: 1"));
    assert!(doctor.contains(&format!(
        "profile_check: profile=homebox registered=true reachable=true server_addr={addr} ssh_target=unknown"
    )));
    assert!(doctor.contains(
        "sync_entry_check: path=private kind=directory profile=homebox profile_registered=true gitignored=true tracked=false local_exists=true"
    ));
    assert!(doctor.contains("doctor_status: ok"));
    assert!(doctor.contains("doctor_error_count: 0"));
    assert!(doctor.contains("doctor_warning_count: 0"));
}

// @verifies PROJECTOR.CLI.DOCTOR.REPORTS_SYNC_ENTRY_SANITY
#[test]
fn doctor_reports_sync_entry_problems() {
    let repo = temp_repo("doctor-problems");
    let projector_home = temp_projector_home("doctor-problems");
    let projector_home_str = projector_home.to_str().expect("projector home utf8");
    fs::create_dir_all(repo.join("private")).expect("create private dir");
    fs::write(repo.join("private/note.txt"), "tracked\n").expect("write tracked file");
    let git_add = Command::new("git")
        .arg("add")
        .arg("private/note.txt")
        .current_dir(&repo)
        .status()
        .expect("git add");
    assert!(git_add.success(), "git add failed");

    FileRepoSyncConfigStore::new(&repo)
        .save(&RepoSyncConfig {
            entries: vec![RepoSyncEntry {
                entry_id: "entry-private-note".to_owned(),
                workspace_id: WorkspaceId::new("ws-doctor"),
                actor_id: ActorId::new("actor-doctor"),
                server_profile_id: "missingbox".to_owned(),
                local_relative_path: PathBuf::from("private/note.txt"),
                remote_relative_path: PathBuf::from("private/note.txt"),
                kind: SyncEntryKind::File,
            }],
        })
        .expect("save sync config");
    FileRuntimeStatusStore::new(repo.join(".projector/status.txt"))
        .save(&RuntimeStatus {
            sync_issue_count: 1,
            last_sync_issue_code: Some("stale_cursor".to_owned()),
            last_sync_issue_disposition: Some(SyncIssueDisposition::NeedsRebootstrap),
            last_sync_issue: Some("stale cursor".to_owned()),
            ..RuntimeStatus::default()
        })
        .expect("save runtime status");

    let doctor = run_projector_with_env(
        &repo,
        &["doctor"],
        &[("PROJECTOR_HOME", projector_home_str)],
    );

    assert!(doctor.contains("connected_profile_count: 0"));
    assert!(doctor.contains("machine_daemon_running: false"));
    assert!(doctor.contains("repo_registered: false"));
    assert!(doctor.contains("runtime_lease_active: false"));
    assert!(doctor.contains("recent_sync_issue_count: 1"));
    assert!(doctor.contains("sync_entry_count: 1"));
    assert!(doctor.contains(
        "profile_check: profile=missingbox registered=false reachable=unknown server_addr=unknown ssh_target=unknown"
    ));
    assert!(doctor.contains(
        "sync_entry_check: path=private/note.txt kind=file profile=missingbox profile_registered=false gitignored=false tracked=true local_exists=true"
    ));
    assert!(doctor.contains("doctor_status: error"));
    assert!(doctor.contains("doctor_error_count: "));
    assert!(doctor.contains("doctor_warning_count: "));
    assert!(doctor.contains("doctor_error: server profile missingbox is not registered"));
    assert!(doctor.contains("doctor_error: sync entry private/note.txt is not gitignored"));
    assert!(doctor.contains("doctor_error: sync entry private/note.txt is already tracked by git"));
}

// @verifies PROJECTOR.CLI.DOCTOR.REPORTS_RUNTIME_AND_SYNC_ISSUES
#[test]
fn doctor_reports_runtime_and_sync_issue_state() {
    let repo = temp_repo("doctor-runtime");
    let projector_home = temp_projector_home("doctor-runtime");
    let projector_home_str = projector_home.to_str().expect("projector home utf8");
    fs::create_dir_all(repo.join("private")).expect("create private dir");

    FileRepoSyncConfigStore::new(&repo)
        .save(&RepoSyncConfig {
            entries: vec![RepoSyncEntry {
                entry_id: "entry-private".to_owned(),
                workspace_id: WorkspaceId::new("ws-doctor-runtime"),
                actor_id: ActorId::new("actor-doctor-runtime"),
                server_profile_id: "missingbox".to_owned(),
                local_relative_path: PathBuf::from("private"),
                remote_relative_path: PathBuf::from("private"),
                kind: SyncEntryKind::Directory,
            }],
        })
        .expect("save sync config");
    FileRuntimeStatusStore::new(repo.join(".projector/status.txt"))
        .save(&RuntimeStatus {
            sync_issue_count: 1,
            last_sync_issue_code: Some("stale_cursor".to_owned()),
            last_sync_issue_disposition: Some(SyncIssueDisposition::NeedsRebootstrap),
            last_sync_issue: Some("stale cursor".to_owned()),
            ..RuntimeStatus::default()
        })
        .expect("save runtime status");

    let doctor = run_projector_with_env(
        &repo,
        &["doctor"],
        &[("PROJECTOR_HOME", projector_home_str)],
    );

    assert!(doctor.contains("machine_daemon_running: false"));
    assert!(doctor.contains("repo_registered: false"));
    assert!(doctor.contains("runtime_lease_active: false"));
    assert!(doctor.contains("recent_sync_issue_count: 1"));
    assert!(doctor.contains(
        "doctor_warning: repo has sync entries but is not registered in the machine sync registry"
    ));
    assert!(doctor.contains("doctor_warning: repo has 1 recent sync issue(s)"));
}

// @verifies PROJECTOR.CLI.DEPLOY.GUIDED_REMOTE_SETUP
#[test]
fn deploy_guides_remote_sqlite_setup_over_ssh() {
    let repo = temp_repo("deploy-guided");
    let projector_home = temp_projector_home("deploy-guided");
    let (fake_bin, ssh_log, scp_log) = install_fake_ssh_tools(&projector_home);
    let path_env = format!(
        "{}:{}",
        fake_bin.display(),
        std::env::var("PATH").expect("path env")
    );
    let projector_home_str = projector_home.to_str().expect("projector home utf8");
    let output = run_projector_tty_with_env(
        &repo,
        &["deploy"],
        "homebox\nuser@127.0.0.1\n\n\n\n\ny\n",
        &[("PROJECTOR_HOME", projector_home_str), ("PATH", &path_env)],
    );

    assert!(output.contains("deploy_profile: homebox"));
    assert!(output.contains("deploy_backend: sqlite"));
    assert!(output.contains("deploy_isolation: sysbox"));
    assert!(output.contains("deploy_server_addr: 127.0.0.1:8942"));
    assert!(output.contains("deploy_remote_dir: ~/.projector"));
    assert!(output.contains("deploy_sqlite_path: ~/.projector/projector.sqlite3"));
    assert!(output.contains("deploy_builder_image: rust:1.87-bookworm"));
    assert!(output.contains("deploy_container: projector-homebox"));
    assert!(output.contains("deploy_image: debian:bookworm-slim"));
    assert!(output.contains("deploy: complete"));

    let ssh_log = fs::read_to_string(ssh_log).expect("read ssh log");
    assert!(ssh_log.contains("user@127.0.0.1"));
    assert!(ssh_log.contains("docker info --format"));
    assert!(ssh_log.contains("sysbox-runc"));
    assert!(ssh_log.contains("tar -xzf \"$HOME/.projector/projector-source.tar.gz\""));
    assert!(ssh_log.contains("rust:1.87-bookworm"));
    assert!(ssh_log.contains("cargo build --release -p projector-server"));
    assert!(ssh_log.contains("docker pull 'debian:bookworm-slim'"));
    assert!(ssh_log.contains("docker run -d"));
    assert!(ssh_log.contains("--runtime=sysbox-runc"));
    assert!(ssh_log.contains("--name 'projector-homebox'"));
    assert!(ssh_log.contains("-p '0.0.0.0:8942:8942'"));
    assert!(ssh_log.contains("/srv/projector/projector-server"));
    assert!(
        ssh_log.contains(
            "serve --addr '0.0.0.0:8942' --sqlite-path '/srv/projector/projector.sqlite3'"
        )
    );

    let scp_log = fs::read_to_string(scp_log).expect("read scp log");
    assert!(scp_log.contains("projector-source.tar.gz"));
    assert!(scp_log.contains("user@127.0.0.1:~/.projector/projector-source.tar.gz"));
}

// @verifies PROJECTOR.CLI.DEPLOY.USES_SYSBOX_ISOLATION
#[test]
fn deploy_uses_sysbox_isolation_for_default_byo_setup() {
    let repo = temp_repo("deploy-sysbox");
    let projector_home = temp_projector_home("deploy-sysbox");
    let (fake_bin, ssh_log, _scp_log) = install_fake_ssh_tools(&projector_home);
    let path_env = format!(
        "{}:{}",
        fake_bin.display(),
        std::env::var("PATH").expect("path env")
    );
    let projector_home_str = projector_home.to_str().expect("projector home utf8");
    let output = run_projector_with_env(
        &repo,
        &[
            "deploy",
            "--profile",
            "homebox",
            "--ssh",
            "user@127.0.0.1",
            "--yes",
        ],
        &[("PROJECTOR_HOME", projector_home_str), ("PATH", &path_env)],
    );

    assert!(output.contains("isolation: sysbox"));
    assert!(output.contains("container: projector-homebox"));

    let ssh_log = fs::read_to_string(ssh_log).expect("read ssh log");
    assert!(ssh_log.contains("--runtime=sysbox-runc"));
    assert!(ssh_log.contains("docker run -d"));
    assert!(!ssh_log.contains("nohup"));
}

// @verifies PROJECTOR.SERVER.HOSTING.SQLITE_DEFAULT
#[test]
fn deploy_defaults_remote_byo_setup_to_sqlite() {
    let repo = temp_repo("deploy-defaults-sqlite");
    let projector_home = temp_projector_home("deploy-defaults-sqlite");
    let (fake_bin, _ssh_log, _scp_log) = install_fake_ssh_tools(&projector_home);
    let path_env = format!(
        "{}:{}",
        fake_bin.display(),
        std::env::var("PATH").expect("path env")
    );
    let projector_home_str = projector_home.to_str().expect("projector home utf8");
    let output = run_projector_tty_with_env(
        &repo,
        &["deploy", "--profile", "homebox", "--ssh", "user@127.0.0.1"],
        "\n\n\n\ny\n",
        &[("PROJECTOR_HOME", projector_home_str), ("PATH", &path_env)],
    );

    assert!(output.contains("deploy_backend: sqlite"));
    assert!(output.contains("deploy_server_addr: 127.0.0.1:8942"));
    assert!(output.contains("deploy_remote_dir: ~/.projector"));
    assert!(output.contains("deploy_sqlite_path: ~/.projector/projector.sqlite3"));
    assert!(output.contains("deploy_listen_addr: 0.0.0.0:8942"));
}

// @verifies PROJECTOR.CLI.DEPLOY.REGISTERS_SERVER_PROFILE
#[test]
fn deploy_registers_and_selects_server_profile() {
    let repo = temp_repo("deploy-registers-profile");
    let projector_home = temp_projector_home("deploy-registers-profile");
    let (fake_bin, _ssh_log, _scp_log) = install_fake_ssh_tools(&projector_home);
    let path_env = format!(
        "{}:{}",
        fake_bin.display(),
        std::env::var("PATH").expect("path env")
    );
    let projector_home_str = projector_home.to_str().expect("projector home utf8");
    let output = run_projector_with_env(
        &repo,
        &[
            "deploy",
            "--profile",
            "homebox",
            "--ssh",
            "user@127.0.0.1",
            "--yes",
        ],
        &[("PROJECTOR_HOME", projector_home_str), ("PATH", &path_env)],
    );

    assert!(output.contains("deploy: complete"));
    assert!(output.contains("profile: homebox"));
    assert!(output.contains("server_addr: 127.0.0.1:8942"));
    assert!(output.contains("backend: sqlite"));
    assert!(output.contains("isolation: sysbox"));

    let registry = FileServerProfileStore::new(ProjectorHome::new(&projector_home))
        .load()
        .expect("load profile registry");
    assert_eq!(registry.profiles.len(), 1);
    assert_eq!(registry.profiles[0].profile_id, "homebox");
    assert_eq!(registry.profiles[0].server_addr, "127.0.0.1:8942");
    assert_eq!(
        registry.profiles[0].ssh_target.as_deref(),
        Some("user@127.0.0.1")
    );
}

// @verifies PROJECTOR.CLI.CONNECT.PERSISTS_GLOBAL_PROFILE_REGISTRY
#[test]
fn connect_add_persists_global_profile_registry() {
    let repo = temp_repo("connect-persists-registry");
    let projector_home = temp_projector_home("connect-persists-registry");
    let projector_home_str = projector_home.to_str().expect("projector home utf8");

    run_projector_with_env(
        &repo,
        &["connect", "--id", "homebox", "--server", "127.0.0.1:7000"],
        &[("PROJECTOR_HOME", projector_home_str)],
    );

    let registry = FileServerProfileStore::new(ProjectorHome::new(&projector_home))
        .load()
        .expect("load profile registry");
    assert_eq!(registry.profiles.len(), 1);
    assert_eq!(registry.profiles[0].profile_id, "homebox");
    assert_eq!(registry.profiles[0].server_addr, "127.0.0.1:7000");
}

// @verifies PROJECTOR.CLI.CONNECT.PERSISTS_GLOBAL_PROFILE_REGISTRY
#[test]
fn connect_interactively_persists_global_profile_registry() {
    let repo = temp_repo("connect-interactive");
    let projector_home = temp_projector_home("connect-interactive");
    let projector_home_str = projector_home.to_str().expect("projector home utf8");

    let output = run_projector_tty_with_env(
        &repo,
        &["connect"],
        "homebox\n127.0.0.1:7000\nspotless@spotless-2\n",
        &[("PROJECTOR_HOME", projector_home_str)],
    );

    assert!(output.contains("connection: added"));
    assert!(output.contains("profile: homebox"));
    assert!(output.contains("server_addr: 127.0.0.1:7000"));
    assert!(output.contains("ssh_target: spotless@spotless-2"));

    let registry = FileServerProfileStore::new(ProjectorHome::new(&projector_home))
        .load()
        .expect("load profile registry");
    assert_eq!(registry.profiles.len(), 1);
    assert_eq!(registry.profiles[0].profile_id, "homebox");
    assert_eq!(registry.profiles[0].server_addr, "127.0.0.1:7000");
    assert_eq!(
        registry.profiles[0].ssh_target.as_deref(),
        Some("spotless@spotless-2")
    );
}

// @verifies PROJECTOR.CLI.DISCONNECT.REMOVES_CONNECTED_PROFILE
#[test]
fn disconnect_warns_with_affected_paths_and_removes_profile() {
    let repo = temp_repo("disconnect-profile");
    let projector_home = temp_projector_home("disconnect-profile");
    let projector_home_str = projector_home.to_str().expect("projector home utf8");
    fs::write(repo.join(".gitignore"), "private/\n").expect("write gitignore");
    fs::create_dir_all(repo.join("private")).expect("create private dir");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    run_projector_with_env(
        &repo,
        &["connect", "--id", "homebox", "--server", &addr],
        &[("PROJECTOR_HOME", projector_home_str)],
    );
    run_projector_with_env(
        &repo,
        &["add", "private"],
        &[("PROJECTOR_HOME", projector_home_str)],
    );

    let output = run_projector_tty_with_env(
        &repo,
        &["disconnect", "homebox"],
        "y\n",
        &[("PROJECTOR_HOME", projector_home_str)],
    );

    assert!(output.contains("disconnect_profile: homebox"));
    assert!(output.contains("affected_sync_entry_count: 1"));
    assert!(output.contains("affected_sync_entry: repo="));
    assert!(output.contains("path=private"));
    assert!(output.contains("kind=directory"));
    assert!(output.contains("disconnect: complete"));

    let registry = FileServerProfileStore::new(ProjectorHome::new(&projector_home))
        .load()
        .expect("load profile registry");
    assert!(registry.profiles.is_empty());
}

// @verifies PROJECTOR.CLI.ADD
#[test]
fn add_registers_sync_entry_in_repo_local_config() {
    let repo = temp_repo("add-sync-entry");
    let projector_home = temp_projector_home("add-sync-entry");
    let projector_home_str = projector_home.to_str().expect("projector home utf8");
    fs::write(repo.join(".gitignore"), "private/\n").expect("write gitignore");
    fs::create_dir_all(repo.join("private")).expect("create private dir");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();
    run_projector_with_env(
        &repo,
        &["connect", "--id", "homebox", "--server", &addr],
        &[("PROJECTOR_HOME", projector_home_str)],
    );

    let stdout = run_projector_with_env(
        &repo,
        &["add", "private"],
        &[("PROJECTOR_HOME", projector_home_str)],
    );
    let config = FileRepoSyncConfigStore::new(&repo)
        .load()
        .expect("load sync config");

    assert!(stdout.contains("sync_entry: added"));
    assert!(stdout.contains("path: private"));
    assert_eq!(config.entries.len(), 1);
    assert_eq!(config.entries[0].server_profile_id, "homebox");
    assert_eq!(config.entries[0].local_relative_path, Path::new("private"));
    assert_eq!(config.entries[0].remote_relative_path, Path::new("private"));
    assert_eq!(config.entries[0].kind, SyncEntryKind::Directory);
}

// @verifies PROJECTOR.CLI.ADD.BOOTSTRAPS_LOCAL_SYNC_ENTRY
#[test]
fn add_bootstraps_existing_local_sync_entry_against_selected_server() {
    let repo = temp_repo("add-bootstrap");
    let projector_home = temp_projector_home("add-bootstrap");
    let projector_home_str = projector_home.to_str().expect("projector home utf8");
    fs::write(repo.join(".gitignore"), "private/\n").expect("write gitignore");
    fs::create_dir_all(repo.join("private/briefs")).expect("create private dir");
    fs::write(
        repo.join("private/briefs/local-first.html"),
        "<h1>Local First</h1>\n<p>Publish me on add.</p>\n",
    )
    .expect("write local-first file");

    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    run_projector_with_env(
        &repo,
        &["connect", "--id", "homebox", "--server", &addr],
        &[("PROJECTOR_HOME", projector_home_str)],
    );

    let stdout = run_projector_with_env(
        &repo,
        &["add", "private"],
        &[("PROJECTOR_HOME", projector_home_str)],
    );

    assert!(stdout.contains("sync_entry: added"));
    assert!(stdout.contains("path: private"));
    assert!(stdout.contains("server_profile: homebox"));

    let snapshot = fs::read_to_string(
        state_dir
            .join("workspaces")
            .read_dir()
            .expect("workspace dir")
            .next()
            .expect("workspace entry")
            .expect("workspace dir entry")
            .path()
            .join("snapshot.json"),
    )
    .expect("read server snapshot");
    assert!(snapshot.contains("\"mount_relative_path\": \"private\""));
    assert!(snapshot.contains("\"relative_path\": \"briefs/local-first.html\""));
    assert!(snapshot.contains("Local First"));
    assert_eq!(
        fs::read_to_string(repo.join("private/briefs/local-first.html"))
            .expect("read rematerialized file"),
        "<h1>Local First</h1>\n<p>Publish me on add.</p>\n"
    );
}

#[test]
fn add_can_bootstrap_a_second_sync_entry_in_the_same_repo() {
    let repo = temp_repo("add-second-entry");
    let projector_home = temp_projector_home("add-second-entry");
    let projector_home_str = projector_home.to_str().expect("projector home utf8");
    fs::write(repo.join(".gitignore"), "AGENTS.md\n_project/\n").expect("write gitignore");
    fs::write(repo.join("AGENTS.md"), "# Local agent notes\n").expect("write AGENTS");
    fs::create_dir_all(repo.join("_project/notes")).expect("create _project dir");
    fs::write(
        repo.join("_project/notes/plan.md"),
        "# Plan\n\nSecond sync entry.\n",
    )
    .expect("write plan");

    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    run_projector_with_env(
        &repo,
        &["connect", "--id", "homebox", "--server", &addr],
        &[("PROJECTOR_HOME", projector_home_str)],
    );

    let first = run_projector_with_env(
        &repo,
        &["add", "--force", "AGENTS.md"],
        &[("PROJECTOR_HOME", projector_home_str)],
    );
    let second = run_projector_with_env(
        &repo,
        &["add", "_project"],
        &[("PROJECTOR_HOME", projector_home_str)],
    );
    let config = FileRepoSyncConfigStore::new(&repo)
        .load()
        .expect("load sync config");
    let status = run_projector_with_env(
        &repo,
        &["status"],
        &[("PROJECTOR_HOME", projector_home_str)],
    );

    assert!(first.contains("sync_entry: added"));
    assert!(second.contains("sync_entry: added"));
    assert_eq!(config.entries.len(), 2);
    assert!(
        config
            .entries
            .iter()
            .any(|entry| entry.local_relative_path == Path::new("AGENTS.md"))
    );
    assert!(
        config
            .entries
            .iter()
            .any(|entry| entry.local_relative_path == Path::new("_project"))
    );
    assert_eq!(
        fs::read_to_string(repo.join("AGENTS.md")).expect("read AGENTS"),
        "# Local agent notes\n"
    );
    assert_eq!(
        fs::read_to_string(repo.join("_project/notes/plan.md")).expect("read plan"),
        "# Plan\n\nSecond sync entry.\n"
    );
    assert!(!status.contains("last_sync_issue"), "{status}");
    assert!(!status.contains("sync_issue_count:"), "{status}");
}

// @verifies PROJECTOR.BINDING.SERVER_PROFILE
#[test]
fn sync_entries_store_named_server_profile_ids() {
    let repo = temp_repo("binding-server-profile");
    let projector_home = temp_projector_home("binding-server-profile");
    let projector_home_str = projector_home.to_str().expect("projector home utf8");
    fs::write(repo.join(".gitignore"), "private/\n").expect("write gitignore");
    fs::create_dir_all(repo.join("private")).expect("create private dir");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    run_projector_with_env(
        &repo,
        &["connect", "--id", "homebox", "--server", &addr],
        &[("PROJECTOR_HOME", projector_home_str)],
    );
    run_projector_with_env(
        &repo,
        &["add", "private"],
        &[("PROJECTOR_HOME", projector_home_str)],
    );

    let config = FileRepoSyncConfigStore::new(&repo)
        .load()
        .expect("load sync config");
    assert_eq!(config.entries[0].server_profile_id, "homebox");
}

// @verifies PROJECTOR.BINDING.PATH_SCOPED_ENTRIES
#[test]
fn sync_config_stores_multiple_path_scoped_entries() {
    let repo = temp_repo("binding-path-scoped");
    let projector_home = temp_projector_home("binding-path-scoped");
    let projector_home_str = projector_home.to_str().expect("projector home utf8");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    fs::create_dir_all(repo.join("private")).expect("create private dir");
    fs::create_dir_all(repo.join("notes")).expect("create notes dir");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    run_projector_with_env(
        &repo,
        &["connect", "--id", "homebox", "--server", &addr],
        &[("PROJECTOR_HOME", projector_home_str)],
    );
    run_projector_with_env(
        &repo,
        &["add", "private"],
        &[("PROJECTOR_HOME", projector_home_str)],
    );
    run_projector_with_env(
        &repo,
        &["add", "notes"],
        &[("PROJECTOR_HOME", projector_home_str)],
    );

    let config = FileRepoSyncConfigStore::new(&repo)
        .load()
        .expect("load sync config");
    assert_eq!(config.entries.len(), 2);
    assert!(
        config
            .entries
            .iter()
            .any(|entry| entry.local_relative_path == Path::new("private"))
    );
    assert!(
        config
            .entries
            .iter()
            .any(|entry| entry.local_relative_path == Path::new("notes"))
    );
}

// @verifies PROJECTOR.BINDING.ONE_SERVER_PROFILE_PER_ENTRY
#[test]
fn each_sync_entry_refers_to_exactly_one_selected_server_profile() {
    let repo = temp_repo("binding-one-profile-per-entry");
    let projector_home = temp_projector_home("binding-one-profile-per-entry");
    let projector_home_str = projector_home.to_str().expect("projector home utf8");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    fs::create_dir_all(repo.join("private")).expect("create private dir");
    fs::create_dir_all(repo.join("notes")).expect("create notes dir");
    let state_dir_a = repo.join("server-state-homebox");
    let addr_a = spawn_server(&state_dir_a).to_string();
    let state_dir_b = repo.join("server-state-workbox");
    let addr_b = spawn_server(&state_dir_b).to_string();

    run_projector_with_env(
        &repo,
        &["connect", "--id", "homebox", "--server", &addr_a],
        &[("PROJECTOR_HOME", projector_home_str)],
    );
    run_projector_with_env(
        &repo,
        &["connect", "--id", "workbox", "--server", &addr_b],
        &[("PROJECTOR_HOME", projector_home_str)],
    );
    run_projector_with_env(
        &repo,
        &["add", "--profile", "homebox", "private"],
        &[("PROJECTOR_HOME", projector_home_str)],
    );
    run_projector_with_env(
        &repo,
        &["add", "--profile", "workbox", "notes"],
        &[("PROJECTOR_HOME", projector_home_str)],
    );

    let config = FileRepoSyncConfigStore::new(&repo)
        .load()
        .expect("load sync config");
    let private = config
        .entries
        .iter()
        .find(|entry| entry.local_relative_path == Path::new("private"))
        .expect("private entry");
    let notes = config
        .entries
        .iter()
        .find(|entry| entry.local_relative_path == Path::new("notes"))
        .expect("notes entry");
    assert_eq!(private.server_profile_id, "homebox");
    assert_eq!(notes.server_profile_id, "workbox");
}

// @verifies PROJECTOR.CLI.ADD
#[test]
fn add_registers_repo_in_machine_sync_registry() {
    let repo = temp_repo("add-global-registry");
    let projector_home = temp_projector_home("add-global-registry");
    let projector_home_str = projector_home.to_str().expect("projector home utf8");
    fs::write(repo.join(".gitignore"), "private/\n").expect("write gitignore");
    fs::create_dir_all(repo.join("private")).expect("create private dir");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();
    run_projector_with_env(
        &repo,
        &["connect", "--id", "homebox", "--server", &addr],
        &[("PROJECTOR_HOME", projector_home_str)],
    );

    run_projector_with_env(
        &repo,
        &["add", "private"],
        &[("PROJECTOR_HOME", projector_home_str)],
    );

    let registry = FileMachineSyncRegistryStore::new(ProjectorHome::new(&projector_home))
        .load()
        .expect("load machine sync registry");
    let expected_repo_root = repo.canonicalize().expect("canonicalize repo root");
    assert_eq!(registry.repos.len(), 1);
    assert_eq!(registry.repos[0].repo_root, expected_repo_root);
    assert_eq!(registry.repos[0].entry_count, 1);
}

// @verifies PROJECTOR.CLI.ADD.REJECTS_VERSION_CONTROLLED_PATH_WITHOUT_FORCE
#[test]
fn add_rejects_version_controlled_path_without_force() {
    let repo = temp_repo("add-tracked");
    let projector_home = temp_projector_home("add-tracked");
    let projector_home_str = projector_home.to_str().expect("projector home utf8");
    fs::write(repo.join(".gitignore"), "private/\ntracked.html\n").expect("write gitignore");
    fs::write(repo.join("tracked.html"), "<p>tracked</p>\n").expect("write tracked file");
    let status = Command::new("git")
        .args(["add", "-f", "tracked.html"])
        .current_dir(&repo)
        .status()
        .expect("git add tracked file");
    assert!(status.success(), "git add failed");

    let stderr = run_projector_failure_with_env(
        &repo,
        &["add", "tracked.html"],
        &[("PROJECTOR_HOME", projector_home_str)],
    );

    assert!(stderr.contains("already under version control"));
    assert!(stderr.contains("--force"));
}

// @verifies PROJECTOR.CLI.ADD.REQUIRES_CONNECTED_SERVER_PROFILE
#[test]
fn add_requires_connected_server_profile_for_new_sync_entry() {
    let repo = temp_repo("add-requires-profile");
    let projector_home = temp_projector_home("add-requires-profile");
    let projector_home_str = projector_home.to_str().expect("projector home utf8");
    fs::write(repo.join(".gitignore"), "private/\n").expect("write gitignore");
    fs::create_dir_all(repo.join("private")).expect("create private dir");

    let stderr = run_projector_failure_with_env(
        &repo,
        &["add", "private"],
        &[("PROJECTOR_HOME", projector_home_str)],
    );

    assert!(stderr.contains("no server profiles are connected"));
    assert!(stderr.contains("projector connect"));
}

// @verifies PROJECTOR.SERVER.SYNC_ENTRIES.LIST
#[test]
fn server_lists_remote_sync_entries_with_metadata_and_preview() {
    let repo = temp_repo("server-list-sync-entries");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();
    let snapshot = BootstrapSnapshot {
        manifest: ManifestState {
            entries: vec![ManifestEntry {
                document_id: DocumentId::new("doc-reference"),
                mount_relative_path: PathBuf::from("reference.html"),
                relative_path: PathBuf::new(),
                kind: DocumentKind::Text,
                deleted: false,
            }],
        },
        bodies: vec![DocumentBody {
            document_id: DocumentId::new("doc-reference"),
            text: "<h1>Remote Reference</h1>\n<p>Preview me.</p>\n".to_owned(),
        }],
    };
    seed_remote_sync_entry(
        &state_dir,
        "ws-remote-reference",
        "reference.html",
        SyncEntryKind::File,
        "source-repo",
        &snapshot,
    );

    let transport = HttpTransport::new(format!("http://{addr}"));
    let entries = transport
        .list_sync_entries(10)
        .expect("list remote sync entries");

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].sync_entry_id, "ws-remote-reference");
    assert_eq!(entries[0].workspace_id, "ws-remote-reference");
    assert_eq!(entries[0].remote_path, "reference.html");
    assert_eq!(entries[0].kind, SyncEntryKind::File);
    assert_eq!(entries[0].source_repo_name.as_deref(), Some("source-repo"));
    assert!(
        entries[0]
            .preview
            .as_deref()
            .expect("preview")
            .contains("Remote Reference")
    );
}

// @verifies PROJECTOR.CLI.GET.BY_ID
#[test]
fn get_by_id_attaches_remote_sync_entry_and_materializes_it_locally() {
    let repo = temp_repo("get-by-id");
    let projector_home = temp_projector_home("get-by-id");
    let projector_home_str = projector_home.to_str().expect("projector home utf8");
    fs::write(repo.join(".gitignore"), "retrieved/\n").expect("write gitignore");

    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();
    let snapshot = BootstrapSnapshot {
        manifest: ManifestState {
            entries: vec![ManifestEntry {
                document_id: DocumentId::new("doc-remote-brief"),
                mount_relative_path: PathBuf::from("private"),
                relative_path: PathBuf::from("briefs/seeded.html"),
                kind: DocumentKind::Text,
                deleted: false,
            }],
        },
        bodies: vec![DocumentBody {
            document_id: DocumentId::new("doc-remote-brief"),
            text: "<h1>Seeded remote brief</h1>\n<p>Fetched through get.</p>\n".to_owned(),
        }],
    };
    seed_remote_sync_entry(
        &state_dir,
        "ws-remote-dir",
        "private",
        SyncEntryKind::Directory,
        "source-repo",
        &snapshot,
    );

    run_projector_with_env(
        &repo,
        &["connect", "--id", "homebox", "--server", &addr],
        &[("PROJECTOR_HOME", projector_home_str)],
    );

    let transport = HttpTransport::new(format!("http://{addr}"));
    let sync_entry_id = transport.list_sync_entries(10).expect("list sync entries")[0]
        .sync_entry_id
        .clone();

    let stdout = run_projector_with_env(
        &repo,
        &["get", &sync_entry_id, "retrieved"],
        &[("PROJECTOR_HOME", projector_home_str)],
    );

    assert!(stdout.contains("sync_entry: retrieved"));
    assert!(stdout.contains(&format!("sync_entry_id: {sync_entry_id}")));
    assert!(stdout.contains("server_profile: homebox"));
    assert!(stdout.contains("remote_path: private"));
    assert!(stdout.contains("local_path: retrieved"));

    let config = FileRepoSyncConfigStore::new(&repo)
        .load()
        .expect("load sync config");
    assert_eq!(config.entries.len(), 1);
    assert_eq!(config.entries[0].entry_id, sync_entry_id);
    assert_eq!(config.entries[0].workspace_id.as_str(), "ws-remote-dir");
    assert_eq!(config.entries[0].server_profile_id, "homebox");
    assert_eq!(
        config.entries[0].local_relative_path,
        Path::new("retrieved")
    );
    assert_eq!(config.entries[0].remote_relative_path, Path::new("private"));
    assert_eq!(config.entries[0].kind, SyncEntryKind::Directory);
    assert_eq!(
        fs::read_to_string(repo.join("retrieved/briefs/seeded.html"))
            .expect("read materialized file"),
        "<h1>Seeded remote brief</h1>\n<p>Fetched through get.</p>\n"
    );
}

// @verifies PROJECTOR.BINDING.WHOLE_REMOTE_ENTRY
#[test]
fn get_attaches_one_whole_remote_sync_entry_by_stable_server_id() {
    let repo = temp_repo("get-whole-entry");
    let projector_home = temp_projector_home("get-whole-entry");
    let projector_home_str = projector_home.to_str().expect("projector home utf8");
    fs::write(repo.join(".gitignore"), "mirror/\n").expect("write gitignore");

    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();
    let snapshot = BootstrapSnapshot {
        manifest: ManifestState {
            entries: vec![ManifestEntry {
                document_id: DocumentId::new("doc-whole-entry"),
                mount_relative_path: PathBuf::from("private"),
                relative_path: PathBuf::from("briefs/entry.html"),
                kind: DocumentKind::Text,
                deleted: false,
            }],
        },
        bodies: vec![DocumentBody {
            document_id: DocumentId::new("doc-whole-entry"),
            text: "<p>whole entry</p>\n".to_owned(),
        }],
    };
    seed_remote_sync_entry(
        &state_dir,
        "ws-whole-entry",
        "private",
        SyncEntryKind::Directory,
        "seed-repo",
        &snapshot,
    );

    run_projector_with_env(
        &repo,
        &["connect", "--id", "homebox", "--server", &addr],
        &[("PROJECTOR_HOME", projector_home_str)],
    );

    let transport = HttpTransport::new(format!("http://{addr}"));
    let sync_entry_id = transport.list_sync_entries(10).expect("list sync entries")[0]
        .sync_entry_id
        .clone();

    run_projector_with_env(
        &repo,
        &["get", &sync_entry_id, "mirror"],
        &[("PROJECTOR_HOME", projector_home_str)],
    );

    let config = FileRepoSyncConfigStore::new(&repo)
        .load()
        .expect("load sync config");
    assert_eq!(config.entries.len(), 1);
    assert_eq!(config.entries[0].entry_id, sync_entry_id);
    assert_eq!(config.entries[0].local_relative_path, Path::new("mirror"));
    assert_eq!(config.entries[0].remote_relative_path, Path::new("private"));
    assert!(repo.join("mirror/briefs/entry.html").is_file());
    assert!(!repo.join("mirror/entry.html").exists());
}

// @verifies PROJECTOR.CLI.GET.BROWSER
#[test]
fn get_without_id_opens_browser_with_metadata_and_preview_before_materializing() {
    let repo = temp_repo("get-browser");
    let projector_home = temp_projector_home("get-browser");
    let projector_home_str = projector_home.to_str().expect("projector home utf8");
    fs::write(repo.join(".gitignore"), "reference.html\n").expect("write gitignore");

    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();
    let snapshot = BootstrapSnapshot {
        manifest: ManifestState {
            entries: vec![ManifestEntry {
                document_id: DocumentId::new("doc-browser-reference"),
                mount_relative_path: PathBuf::from("reference.html"),
                relative_path: PathBuf::new(),
                kind: DocumentKind::Text,
                deleted: false,
            }],
        },
        bodies: vec![DocumentBody {
            document_id: DocumentId::new("doc-browser-reference"),
            text: "<h1>Remote Reference</h1>\n<p>Preview me.</p>\n".to_owned(),
        }],
    };
    seed_remote_sync_entry(
        &state_dir,
        "ws-browser-reference",
        "reference.html",
        SyncEntryKind::File,
        "browser-source-repo",
        &snapshot,
    );

    run_projector_with_env(
        &repo,
        &["connect", "--id", "homebox", "--server", &addr],
        &[("PROJECTOR_HOME", projector_home_str)],
    );

    let output = run_projector_tty_with_env(
        &repo,
        &["get"],
        "\r",
        &[("PROJECTOR_HOME", projector_home_str)],
    );

    assert!(output.contains("ws-browser-reference"));
    assert!(output.contains("source_repo: browser-source-repo"));
    assert!(output.contains("preview:"));
    assert!(output.contains("Remote Reference"));
    assert!(output.contains("sync_entry: retrieved"));
    assert!(output.contains("local_path: reference.html"));
    assert_eq!(
        fs::read_to_string(repo.join("reference.html")).expect("read fetched file"),
        "<h1>Remote Reference</h1>\n<p>Preview me.</p>\n"
    );
}

// @verifies PROJECTOR.CLI.REMOVE
#[test]
fn remove_and_rm_delete_sync_entry_from_repo_local_config() {
    let repo = temp_repo("remove-sync-entry");
    let projector_home = temp_projector_home("remove-sync-entry");
    let projector_home_str = projector_home.to_str().expect("projector home utf8");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    fs::create_dir_all(repo.join("private")).expect("create private dir");
    fs::create_dir_all(repo.join("notes")).expect("create notes dir");
    let state_dir = projector_home.join("server-state");
    fs::create_dir_all(&state_dir).expect("create server state dir");
    let addr = spawn_server(&state_dir).to_string();
    run_projector_with_env(
        &repo,
        &["connect", "--id", "homebox", "--server", &addr],
        &[("PROJECTOR_HOME", projector_home_str)],
    );

    run_projector_with_env(
        &repo,
        &["add", "private"],
        &[("PROJECTOR_HOME", projector_home_str)],
    );
    run_projector_with_env(
        &repo,
        &["add", "notes"],
        &[("PROJECTOR_HOME", projector_home_str)],
    );

    let remove_stdout = run_projector_with_env(
        &repo,
        &["remove", "private"],
        &[("PROJECTOR_HOME", projector_home_str)],
    );
    let rm_stdout = run_projector_with_env(
        &repo,
        &["rm", "notes"],
        &[("PROJECTOR_HOME", projector_home_str)],
    );
    let config = FileRepoSyncConfigStore::new(&repo)
        .load()
        .expect("load sync config");

    assert!(remove_stdout.contains("sync_entry: removed"));
    assert!(remove_stdout.contains("path: private"));
    assert!(rm_stdout.contains("sync_entry: removed"));
    assert!(rm_stdout.contains("path: notes"));
    assert_eq!(config, RepoSyncConfig::default());
}

// @verifies PROJECTOR.CLI.REMOVE
#[test]
fn remove_unregisters_repo_when_last_sync_entry_is_removed() {
    let repo = temp_repo("remove-global-registry");
    let projector_home = temp_projector_home("remove-global-registry");
    let projector_home_str = projector_home.to_str().expect("projector home utf8");
    fs::write(repo.join(".gitignore"), "private/\n").expect("write gitignore");
    fs::create_dir_all(repo.join("private")).expect("create private dir");
    let state_dir = projector_home.join("server-state");
    fs::create_dir_all(&state_dir).expect("create server state dir");
    let addr = spawn_server(&state_dir).to_string();
    run_projector_with_env(
        &repo,
        &["connect", "--id", "homebox", "--server", &addr],
        &[("PROJECTOR_HOME", projector_home_str)],
    );

    run_projector_with_env(
        &repo,
        &["add", "private"],
        &[("PROJECTOR_HOME", projector_home_str)],
    );
    run_projector_with_env(
        &repo,
        &["remove", "private"],
        &[("PROJECTOR_HOME", projector_home_str)],
    );

    let registry = FileMachineSyncRegistryStore::new(ProjectorHome::new(&projector_home))
        .load()
        .expect("load machine sync registry");
    assert!(registry.repos.is_empty());
}

// @verifies PROJECTOR.CLI.SYNC.MANAGES_MACHINE_DAEMON_PROCESS
#[test]
fn sync_start_status_and_stop_manage_machine_daemon() {
    let repo = temp_repo("machine-daemon");
    let projector_home = temp_projector_home("machine-daemon");
    let projector_home_str = projector_home.to_str().expect("projector home utf8");

    let start = run_projector_with_env(
        &repo,
        &["sync", "start"],
        &[
            ("PROJECTOR_HOME", projector_home_str),
            ("PROJECTOR_DAEMON_POLL_MS", "50"),
        ],
    );
    assert!(start.contains("daemon_running: true"));

    let status = run_projector_with_env(
        &repo,
        &["sync", "status"],
        &[("PROJECTOR_HOME", projector_home_str)],
    );
    assert!(status.contains("daemon_running: true"));
    assert!(status.contains(&format!("projector_home: {}", projector_home.display())));

    let stop = run_projector_with_env(
        &repo,
        &["sync", "stop"],
        &[("PROJECTOR_HOME", projector_home_str)],
    );
    assert!(stop.contains("daemon_running: false"));

    let status = run_projector_with_env(
        &repo,
        &["sync", "status"],
        &[("PROJECTOR_HOME", projector_home_str)],
    );
    assert!(status.contains("daemon_running: false"));
}

#[test]
fn status_and_log_surface_local_sync_issues() {
    let repo = temp_repo("sync-issue-status");
    let binding = CheckoutBinding {
        workspace_id: WorkspaceId::new("ws-sync-issue"),
        actor_id: ActorId::new("actor-sync-issue"),
        server_addr: None,
        roots: ProjectionRoots {
            projector_dir: repo.join(".projector"),
            projection_paths: vec![repo.join("private"), repo.join("notes")],
        },
        projection_relative_paths: vec![PathBuf::from("private"), PathBuf::from("notes")],
        projection_kinds: vec![SyncEntryKind::Directory, SyncEntryKind::Directory],
    };
    FileBindingStore::new(&repo)
        .save(&binding)
        .expect("save binding");
    save_sync_config_for_binding(&repo, &binding);
    fs::create_dir_all(repo.join("private/briefs")).expect("create projection dir");
    fs::write(
        repo.join("private/briefs/conflict.html"),
        "<p>needs create</p>\n",
    )
    .expect("write local file");

    let mut runner = SyncRunner::new(&binding, Some(RejectCreateTransport));
    let err = runner
        .run(&SyncLoopOptions {
            watch: false,
            poll_ms: 50,
            watch_cycles: None,
        })
        .expect_err("sync should record local issue");
    assert!(
        err.to_string().contains("stale_cursor"),
        "unexpected error: {err}"
    );

    let status = run_projector(&repo, &["status"]);
    let log = run_projector(&repo, &["log"]);

    assert!(status.contains("recovery_attempt_count: 1"));
    assert!(status.contains("last_recovery_action: needs_rebootstrap_retry"));
    assert!(status.contains("sync_issue_count: 1"));
    assert!(status.contains("last_sync_issue_code: stale_cursor"));
    assert!(status.contains("last_sync_issue_disposition: needs_rebootstrap"));
    assert!(status.contains(
        "last_sync_issue: create document request failed with status 409 Conflict: stale_cursor"
    ));
    assert!(log.contains("kind=sync_recovery"));
    assert!(log.contains("action=needs_rebootstrap_retry attempt=1"));
    assert!(log.contains("kind=sync_issue"));
    assert!(log.contains("disposition=needs_rebootstrap"));
    assert!(log.contains("code=stale_cursor"));
    assert!(log.contains("stale_cursor"));
}

#[test]
fn sync_retries_rebootstrap_required_issue_once_before_recording_it() {
    let repo = temp_repo("retry-rebootstrap");
    let binding = CheckoutBinding {
        workspace_id: WorkspaceId::new("ws-retry"),
        actor_id: ActorId::new("actor-retry"),
        server_addr: None,
        roots: ProjectionRoots {
            projector_dir: repo.join(".projector"),
            projection_paths: vec![repo.join("private"), repo.join("notes")],
        },
        projection_relative_paths: vec![PathBuf::from("private"), PathBuf::from("notes")],
        projection_kinds: vec![SyncEntryKind::Directory, SyncEntryKind::Directory],
    };
    FileBindingStore::new(&repo)
        .save(&binding)
        .expect("save binding");
    save_sync_config_for_binding(&repo, &binding);
    fs::create_dir_all(repo.join("private/briefs")).expect("create projection dir");
    fs::write(repo.join("private/briefs/retry.html"), "<p>retry me</p>\n")
        .expect("write local file");

    let mut runner = SyncRunner::new(&binding, Some(RetryAfterRebootstrapTransport::default()));
    runner
        .run(&SyncLoopOptions {
            watch: false,
            poll_ms: 50,
            watch_cycles: None,
        })
        .expect("sync should recover after rebootstrap retry");

    let status = run_projector(&repo, &["status"]);
    let log = run_projector(&repo, &["log"]);

    assert!(status.contains("recovery_attempt_count: 1"));
    assert!(status.contains("last_recovery_action: needs_rebootstrap_retry"));
    assert!(!status.contains("sync_issue_count:"));
    assert!(!status.contains("last_sync_issue_code:"));
    assert!(!status.contains("last_sync_issue_disposition:"));
    assert!(log.contains("kind=sync_recovery"));
    assert!(log.contains("action=needs_rebootstrap_retry attempt=1"));
    assert!(!log.contains("kind=sync_issue"));
}

#[test]
fn sync_retries_transient_transport_issue_before_recording_it() {
    let repo = temp_repo("retry-immediately");
    let binding = CheckoutBinding {
        workspace_id: WorkspaceId::new("ws-retry-immediately"),
        actor_id: ActorId::new("actor-retry-immediately"),
        server_addr: None,
        roots: ProjectionRoots {
            projector_dir: repo.join(".projector"),
            projection_paths: vec![repo.join("private"), repo.join("notes")],
        },
        projection_relative_paths: vec![PathBuf::from("private"), PathBuf::from("notes")],
        projection_kinds: vec![SyncEntryKind::Directory, SyncEntryKind::Directory],
    };
    FileBindingStore::new(&repo)
        .save(&binding)
        .expect("save binding");
    save_sync_config_for_binding(&repo, &binding);
    fs::create_dir_all(repo.join("private/briefs")).expect("create projection dir");
    fs::write(
        repo.join("private/briefs/transient.html"),
        "<p>transient retry</p>\n",
    )
    .expect("write local file");

    let mut runner = SyncRunner::new(&binding, Some(RetryImmediatelyTransport::default()));
    runner
        .run(&SyncLoopOptions {
            watch: false,
            poll_ms: 50,
            watch_cycles: None,
        })
        .expect("sync should recover after transient retry");

    let status = run_projector(&repo, &["status"]);
    let log = run_projector(&repo, &["log"]);

    assert!(status.contains("recovery_attempt_count: 1"));
    assert!(status.contains("last_recovery_action: retry_immediately"));
    assert!(!status.contains("sync_issue_count:"));
    assert!(!status.contains("last_sync_issue_code:"));
    assert!(!status.contains("last_sync_issue_disposition:"));
    assert!(log.contains("kind=sync_recovery"));
    assert!(log.contains("action=retry_immediately attempt=1"));
    assert!(!log.contains("kind=sync_issue"));
}

#[test]
fn watch_status_preserves_recovery_visibility_after_transient_retry() {
    let repo = temp_repo("watch-recovery-status");
    let binding = CheckoutBinding {
        workspace_id: WorkspaceId::new("ws-watch-recovery"),
        actor_id: ActorId::new("actor-watch-recovery"),
        server_addr: None,
        roots: ProjectionRoots {
            projector_dir: repo.join(".projector"),
            projection_paths: vec![repo.join("private"), repo.join("notes")],
        },
        projection_relative_paths: vec![PathBuf::from("private"), PathBuf::from("notes")],
        projection_kinds: vec![SyncEntryKind::Directory, SyncEntryKind::Directory],
    };
    FileBindingStore::new(&repo)
        .save(&binding)
        .expect("save binding");
    save_sync_config_for_binding(&repo, &binding);
    fs::create_dir_all(repo.join("private/briefs")).expect("create projection dir");
    fs::write(
        repo.join("private/briefs/transient.html"),
        "<p>watch retry</p>\n",
    )
    .expect("write local file");

    let mut runner = SyncRunner::new(&binding, Some(RetryImmediatelyTransport::default()));
    runner
        .run(&SyncLoopOptions {
            watch: true,
            poll_ms: 10,
            watch_cycles: Some(2),
        })
        .expect("watch sync should recover after transient retry");

    let status = run_projector(&repo, &["status"]);
    let log = run_projector(&repo, &["log"]);

    assert!(status.contains("recovery_attempt_count: 1"));
    assert!(status.contains("last_recovery_action: retry_immediately"));
    assert!(!status.contains("sync_issue_count:"));
    assert!(log.contains("kind=sync_recovery"));
    assert!(log.contains("action=retry_immediately attempt=1"));
}

// @verifies PROJECTOR.CLI.LOG.RENDERS_LOCAL_EVENTS
#[test]
fn log_renders_local_bootstrap_events() {
    let repo = temp_repo("log");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();
    run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);

    let log = run_projector(&repo, &["log"]);

    assert!(log.contains("kind=sync_bootstrapped"));
    assert!(log.contains("path=private,notes") || log.contains("path=notes,private"));
}

// @verifies PROJECTOR.CLI.LOG.SUMMARY
#[test]
fn log_renders_server_workspace_events() {
    let repo = temp_repo("server-log");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    fs::create_dir_all(repo.join("private/briefs")).expect("create local subdir");
    fs::write(
        repo.join("private/briefs/index.html"),
        "<p>server log me</p>\n",
    )
    .expect("write local text file");

    run_projector(&repo, &["sync"]);
    let log = run_projector(&repo, &["log"]);

    assert!(log.contains("kind=document_created"));
    assert!(log.contains("document_id=doc-"));
    assert!(log.contains("path=private/briefs/index.html"));
    assert!(log.contains("summary=created text document at private/briefs/index.html"));
}

// @verifies PROJECTOR.CLI.STATUS.REPORTS_CONFLICTED_TEXT_DOCUMENTS
#[test]
fn status_reports_conflicted_text_documents_when_file_contains_conflict_markers() {
    let repo = temp_repo("status-conflict-marker");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");

    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    fs::create_dir_all(repo.join("private/briefs")).expect("create base dir");
    fs::write(
        repo.join("private/briefs/conflict.html"),
        "<<<<<<< existing\n<p>repo a edit</p>\n=======\n<p>repo b edit</p>\n>>>>>>> incoming\n",
    )
    .expect("write conflicted file");

    let status = run_projector(&repo, &["status"]);
    assert!(status.contains("conflicted_text_documents: 1"));
    assert!(status.contains("conflicted_text_path: private/briefs/conflict.html"));
}

// @verifies PROJECTOR.CLI.LOG.SUMMARY
#[test]
fn log_surfaces_conflicting_merge_summary_from_server_provenance() {
    let repo_a = temp_repo("log-conflict-a");
    let repo_b = temp_repo("log-conflict-b");
    fs::write(repo_a.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    fs::write(repo_b.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");

    let state_dir = repo_a.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    run_projector(&repo_a, &["sync", "--server", &addr, "private", "notes"]);
    clone_sync_config_for_repo(&repo_a, &repo_b, "actor-log-conflict-b");

    fs::create_dir_all(repo_a.join("private/briefs")).expect("create base dir");
    fs::write(
        repo_a.join("private/briefs/conflict-log.html"),
        "<p>shared base</p>\n",
    )
    .expect("write base file");
    run_projector(&repo_a, &["sync"]);
    run_projector(&repo_b, &["sync"]);

    fs::write(
        repo_a.join("private/briefs/conflict-log.html"),
        "<p>repo a edit</p>\n",
    )
    .expect("write repo a edit");
    fs::write(
        repo_b.join("private/briefs/conflict-log.html"),
        "<p>repo b edit</p>\n",
    )
    .expect("write repo b edit");
    run_projector(&repo_a, &["sync"]);
    run_projector(&repo_b, &["sync"]);

    let log = run_projector(&repo_b, &["log"]);
    assert!(log.contains("kind=document_updated"));
    assert!(log.contains("path=private/briefs/conflict-log.html"));
    assert!(log.contains("merged concurrent text update"));
}

// @verifies PROJECTOR.SERVER.SYNC.CHANGES_SINCE_RETURNS_CHANGED_DOCUMENTS
#[test]
fn changes_since_returns_only_changed_documents() {
    let repo = temp_repo("changes-since");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    let binding = load_workspace_binding_from_sync_config(&repo);
    let mut transport = HttpTransport::new(format!("http://{addr}"));

    let (initial_snapshot, initial_cursor) = transport.bootstrap(&binding).expect("bootstrap");
    assert!(initial_snapshot.manifest.entries.is_empty());
    assert_eq!(initial_cursor, 0);

    let document_id = transport
        .create_document(
            &binding,
            initial_cursor,
            Path::new("private"),
            Path::new("briefs/index.html"),
            "<p>delta create</p>\n",
        )
        .expect("create document");

    let (delta_snapshot, next_cursor) = transport
        .changes_since(&binding, initial_cursor)
        .expect("delta after create");
    assert!(next_cursor > initial_cursor);
    assert_eq!(delta_snapshot.manifest.entries.len(), 1);
    assert_eq!(delta_snapshot.bodies.len(), 1);
    assert_eq!(delta_snapshot.manifest.entries[0].document_id, document_id);
    assert_eq!(delta_snapshot.bodies[0].document_id, document_id);
    assert_eq!(delta_snapshot.bodies[0].text, "<p>delta create</p>\n");

    let (empty_delta, stable_cursor) = transport
        .changes_since(&binding, next_cursor)
        .expect("empty delta");
    assert!(empty_delta.manifest.entries.is_empty());
    assert!(empty_delta.bodies.is_empty());
    assert_eq!(stable_cursor, next_cursor);
}

// @verifies PROJECTOR.SERVER.DOCUMENTS.REJECTS_STALE_MANIFEST_WRITES
#[test]
fn stale_manifest_writes_return_conflict_errors() {
    let repo = temp_repo("stale-manifest");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    let binding = load_workspace_binding_from_sync_config(&repo);
    let mut transport = HttpTransport::new(format!("http://{addr}"));

    let (_snapshot, initial_cursor) = transport.bootstrap(&binding).expect("bootstrap");
    transport
        .create_document(
            &binding,
            initial_cursor,
            Path::new("private"),
            Path::new("briefs/current.html"),
            "<p>fresh write</p>\n",
        )
        .expect("initial manifest write");

    let err = transport
        .create_document(
            &binding,
            initial_cursor,
            Path::new("private"),
            Path::new("briefs/stale.html"),
            "<p>stale write</p>\n",
        )
        .expect_err("stale manifest write should fail");

    let message = err.to_string();
    assert!(
        message.contains("409 Conflict"),
        "unexpected error: {message}"
    );
    assert!(
        message.contains("stale_cursor"),
        "unexpected error: {message}"
    );
}

#[test]
fn sync_bootstraps_bound_workspace_against_server() {
    let repo = temp_repo("server-sync");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir);

    let addr = addr.to_string();
    let stdout = run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);

    let metadata = fs::read_to_string(
        state_dir
            .join("workspaces")
            .join(
                stdout
                    .lines()
                    .find_map(|line| line.strip_prefix("workspace_id: "))
                    .expect("workspace id"),
            )
            .join("metadata.txt"),
    )
    .expect("read server metadata");

    assert!(stdout.contains(&format!("server_addr: {addr}")));
    assert!(metadata.contains("projection_relative_path=private"));
    assert!(metadata.contains("projection_relative_path=notes"));
}

#[test]
fn sync_applies_initial_snapshot_from_server() {
    let repo = temp_repo("snapshot-sync");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir);

    let addr = addr.to_string();
    let first_sync = run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    let workspace_id = first_sync
        .lines()
        .find_map(|line| line.strip_prefix("workspace_id: "))
        .expect("workspace id")
        .to_owned();

    let snapshot = BootstrapSnapshot {
        manifest: ManifestState {
            entries: vec![ManifestEntry {
                document_id: DocumentId::new("doc-1"),
                mount_relative_path: PathBuf::from("private"),
                relative_path: PathBuf::from("briefs/today.md"),
                kind: DocumentKind::Text,
                deleted: false,
            }],
        },
        bodies: vec![DocumentBody {
            document_id: DocumentId::new("doc-1"),
            text: "# Today\n\nShip the bootstrap path.\n".to_owned(),
        }],
    };
    projector_server::write_workspace_snapshot(&state_dir, &workspace_id, &snapshot)
        .expect("write workspace snapshot");

    let second_sync = run_projector(&repo, &["sync"]);

    assert!(second_sync.contains("binding: reused"));
    assert_eq!(
        fs::read_to_string(repo.join("private/briefs/today.md")).expect("read materialized file"),
        "# Today\n\nShip the bootstrap path.\n"
    );
}

#[test]
fn sync_pushes_local_text_creations_to_server() {
    let repo = temp_repo("local-create-sync");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    fs::create_dir_all(repo.join("private/briefs")).expect("create local subdir");
    fs::write(
        repo.join("private/briefs/index.html"),
        "<h1>Local</h1>\n<p>Created before the next sync.</p>\n",
    )
    .expect("write local text file");

    let second_sync = run_projector(&repo, &["sync"]);
    let snapshot = fs::read_to_string(
        state_dir
            .join("workspaces")
            .read_dir()
            .expect("workspace dir")
            .next()
            .expect("workspace entry")
            .expect("workspace dir entry")
            .path()
            .join("snapshot.json"),
    )
    .expect("read server snapshot");
    let log = run_projector(&repo, &["log"]);

    assert!(second_sync.contains("binding: reused"));
    assert!(snapshot.contains("\"mount_relative_path\": \"private\""));
    assert!(snapshot.contains("\"relative_path\": \"briefs/index.html\""));
    assert!(snapshot.contains("Created before the next sync."));
    assert!(log.contains("kind=document_created"));
    assert!(log.contains("path=private/briefs/index.html"));
}

// @verifies PROJECTOR.WORKSPACE.TEXT_ONLY
#[test]
fn sync_ignores_non_utf8_files_under_projection_mounts() {
    let repo = temp_repo("text-only");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    fs::create_dir_all(repo.join("private/assets")).expect("create local subdir");
    fs::write(repo.join("private/assets/blob.bin"), [0, 159, 146, 150]).expect("write binary");
    fs::write(
        repo.join("private/assets/readme.txt"),
        "still text and should sync\n",
    )
    .expect("write text file");

    run_projector(&repo, &["sync"]);

    let binding = load_workspace_binding_from_sync_config(&repo);
    let mut transport = HttpTransport::new(format!("http://{addr}"));
    let (snapshot, _cursor) = transport.bootstrap(&binding).expect("bootstrap");

    assert!(snapshot.manifest.entries.iter().any(|entry| {
        !entry.deleted
            && entry.mount_relative_path == Path::new("private")
            && entry.relative_path == Path::new("assets/readme.txt")
    }));
    assert!(!snapshot.manifest.entries.iter().any(|entry| {
        !entry.deleted
            && entry.mount_relative_path == Path::new("private")
            && entry.relative_path == Path::new("assets/blob.bin")
    }));
}

#[test]
fn sync_pushes_local_text_updates_to_server() {
    let repo = temp_repo("local-update-sync");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    let first_sync = run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    let workspace_id = first_sync
        .lines()
        .find_map(|line| line.strip_prefix("workspace_id: "))
        .expect("workspace id")
        .to_owned();

    let snapshot = BootstrapSnapshot {
        manifest: ManifestState {
            entries: vec![ManifestEntry {
                document_id: DocumentId::new("doc-2"),
                mount_relative_path: PathBuf::from("private"),
                relative_path: PathBuf::from("briefs/index.html"),
                kind: DocumentKind::Text,
                deleted: false,
            }],
        },
        bodies: vec![DocumentBody {
            document_id: DocumentId::new("doc-2"),
            text: "<h1>Old</h1>\n<p>Before edit.</p>\n".to_owned(),
        }],
    };
    projector_server::write_workspace_snapshot(&state_dir, &workspace_id, &snapshot)
        .expect("write workspace snapshot");

    run_projector(&repo, &["sync"]);
    fs::write(
        repo.join("private/briefs/index.html"),
        "<h1>New</h1>\n<p>After edit.</p>\n",
    )
    .expect("write updated local text file");

    let third_sync = run_projector(&repo, &["sync"]);
    let updated_snapshot = fs::read_to_string(
        state_dir
            .join("workspaces")
            .join(&workspace_id)
            .join("snapshot.json"),
    )
    .expect("read server snapshot");
    let log = run_projector(&repo, &["log"]);

    assert!(third_sync.contains("binding: reused"));
    assert!(updated_snapshot.contains("<h1>New</h1>"));
    assert!(updated_snapshot.contains("After edit."));
    assert!(log.contains("kind=document_updated"));
    assert!(log.contains("path=private/briefs/index.html"));
}

#[test]
fn sync_pushes_local_text_deletions_to_server() {
    let repo = temp_repo("local-delete-sync");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    let first_sync = run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    let workspace_id = first_sync
        .lines()
        .find_map(|line| line.strip_prefix("workspace_id: "))
        .expect("workspace id")
        .to_owned();

    let snapshot = BootstrapSnapshot {
        manifest: ManifestState {
            entries: vec![ManifestEntry {
                document_id: DocumentId::new("doc-3"),
                mount_relative_path: PathBuf::from("private"),
                relative_path: PathBuf::from("briefs/index.html"),
                kind: DocumentKind::Text,
                deleted: false,
            }],
        },
        bodies: vec![DocumentBody {
            document_id: DocumentId::new("doc-3"),
            text: "<h1>Delete Me</h1>\n".to_owned(),
        }],
    };
    projector_server::write_workspace_snapshot(&state_dir, &workspace_id, &snapshot)
        .expect("write workspace snapshot");

    run_projector(&repo, &["sync"]);
    fs::remove_file(repo.join("private/briefs/index.html")).expect("remove local file");

    let third_sync = run_projector(&repo, &["sync"]);
    let updated_snapshot = fs::read_to_string(
        state_dir
            .join("workspaces")
            .join(&workspace_id)
            .join("snapshot.json"),
    )
    .expect("read server snapshot");
    let log = run_projector(&repo, &["log"]);

    assert!(third_sync.contains("binding: reused"));
    assert!(updated_snapshot.contains("\"deleted\": true"));
    assert!(log.contains("kind=document_deleted"));
    assert!(log.contains("path=private/briefs/index.html"));
}

#[test]
fn sync_pushes_local_text_moves_to_server() {
    let repo = temp_repo("local-move-sync");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    let first_sync = run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    let workspace_id = first_sync
        .lines()
        .find_map(|line| line.strip_prefix("workspace_id: "))
        .expect("workspace id")
        .to_owned();

    let snapshot = BootstrapSnapshot {
        manifest: ManifestState {
            entries: vec![ManifestEntry {
                document_id: DocumentId::new("doc-4"),
                mount_relative_path: PathBuf::from("private"),
                relative_path: PathBuf::from("briefs/index.html"),
                kind: DocumentKind::Text,
                deleted: false,
            }],
        },
        bodies: vec![DocumentBody {
            document_id: DocumentId::new("doc-4"),
            text: "<h1>Move Me</h1>\n".to_owned(),
        }],
    };
    projector_server::write_workspace_snapshot(&state_dir, &workspace_id, &snapshot)
        .expect("write workspace snapshot");

    run_projector(&repo, &["sync"]);
    fs::create_dir_all(repo.join("private/archive")).expect("create archive dir");
    fs::rename(
        repo.join("private/briefs/index.html"),
        repo.join("private/archive/index.html"),
    )
    .expect("rename local file");

    let third_sync = run_projector(&repo, &["sync"]);
    let updated_snapshot = fs::read_to_string(
        state_dir
            .join("workspaces")
            .join(&workspace_id)
            .join("snapshot.json"),
    )
    .expect("read server snapshot");
    let log = run_projector(&repo, &["log"]);

    assert!(third_sync.contains("binding: reused"));
    assert!(updated_snapshot.contains("\"document_id\": \"doc-4\""));
    assert!(updated_snapshot.contains("\"relative_path\": \"archive/index.html\""));
    assert!(!updated_snapshot.contains("\"relative_path\": \"briefs/index.html\",\n      \"kind\": \"Text\",\n      \"deleted\": false"));
    assert!(repo.join("private/archive/index.html").exists());
    assert!(!repo.join("private/briefs/index.html").exists());
    assert!(log.contains("kind=document_moved"));
    assert!(log.contains("path=private/archive/index.html"));
}

// @verifies PROJECTOR.SYNC.FILE_LIFECYCLE
#[test]
fn sync_reconciles_text_file_lifecycle_through_server_manifest() {
    let repo = temp_repo("file-lifecycle");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    fs::create_dir_all(repo.join("private/briefs")).expect("create local subdir");
    fs::write(
        repo.join("private/briefs/lifecycle.html"),
        "<p>created</p>\n",
    )
    .expect("write initial file");

    run_projector(&repo, &["sync"]);

    let binding = load_workspace_binding_from_sync_config(&repo);
    let mut transport = HttpTransport::new(format!("http://{addr}"));
    let (created_snapshot, created_cursor) =
        transport.bootstrap(&binding).expect("bootstrap create");
    let created_entry = created_snapshot
        .manifest
        .entries
        .iter()
        .find(|entry| {
            !entry.deleted
                && entry.mount_relative_path == Path::new("private")
                && entry.relative_path == Path::new("briefs/lifecycle.html")
        })
        .expect("created manifest entry");
    let created_document_id = created_entry.document_id.clone();
    assert!(
        created_snapshot
            .bodies
            .iter()
            .any(|body| body.document_id == created_document_id && body.text == "<p>created</p>\n")
    );

    fs::write(
        repo.join("private/briefs/lifecycle.html"),
        "<p>updated</p>\n",
    )
    .expect("update file");
    run_projector(&repo, &["sync"]);

    let (updated_snapshot, updated_cursor) = transport
        .changes_since(&binding, created_cursor)
        .expect("delta after update");
    assert!(updated_cursor > created_cursor);
    assert!(
        updated_snapshot
            .bodies
            .iter()
            .any(|body| body.document_id == created_document_id && body.text == "<p>updated</p>\n")
    );

    fs::create_dir_all(repo.join("notes/archive")).expect("create move target");
    fs::rename(
        repo.join("private/briefs/lifecycle.html"),
        repo.join("notes/archive/lifecycle.html"),
    )
    .expect("move file");
    run_projector(&repo, &["sync"]);

    let (moved_snapshot, moved_cursor) = transport
        .changes_since(&binding, updated_cursor)
        .expect("delta after move");
    let moved_entry = moved_snapshot
        .manifest
        .entries
        .iter()
        .find(|entry| entry.document_id == created_document_id)
        .expect("moved manifest entry");
    assert!(!moved_entry.deleted);
    assert_eq!(moved_entry.mount_relative_path, Path::new("notes"));
    assert_eq!(
        moved_entry.relative_path,
        Path::new("archive/lifecycle.html")
    );

    fs::remove_file(repo.join("notes/archive/lifecycle.html")).expect("delete moved file");
    run_projector(&repo, &["sync"]);

    let (deleted_snapshot, deleted_cursor) = transport
        .changes_since(&binding, moved_cursor)
        .expect("delta after delete");
    assert!(deleted_cursor > moved_cursor);
    let deleted_entry = deleted_snapshot
        .manifest
        .entries
        .iter()
        .find(|entry| entry.document_id == created_document_id)
        .expect("deleted manifest entry");
    assert!(deleted_entry.deleted);
    assert!(
        deleted_snapshot
            .bodies
            .iter()
            .all(|body| body.document_id != created_document_id)
    );
}

// @verifies PROJECTOR.PROVENANCE.EVENT_LOG
#[test]
fn server_provenance_records_append_only_file_lifecycle_events() {
    let repo = temp_repo("provenance-events");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    fs::create_dir_all(repo.join("private/briefs")).expect("create local subdir");
    fs::write(
        repo.join("private/briefs/provenance.html"),
        "<p>create</p>\n",
    )
    .expect("write create");
    run_projector(&repo, &["sync"]);

    fs::write(
        repo.join("private/briefs/provenance.html"),
        "<p>update</p>\n",
    )
    .expect("write update");
    run_projector(&repo, &["sync"]);

    fs::create_dir_all(repo.join("notes/archive")).expect("create move target");
    fs::rename(
        repo.join("private/briefs/provenance.html"),
        repo.join("notes/archive/provenance.html"),
    )
    .expect("move file");
    run_projector(&repo, &["sync"]);

    fs::remove_file(repo.join("notes/archive/provenance.html")).expect("delete moved file");
    run_projector(&repo, &["sync"]);

    let binding = load_workspace_binding_from_sync_config(&repo);
    let mut transport = HttpTransport::new(format!("http://{addr}"));
    let events = transport.provenance(&binding, 20).expect("list provenance");

    assert!(events.iter().any(|event| {
        event.kind == projector_domain::ProvenanceEventKind::DocumentCreated
            && event.mount_relative_path.as_deref() == Some("private")
            && event.relative_path.as_deref() == Some("briefs/provenance.html")
            && !event.actor_id.as_str().is_empty()
            && event.timestamp_ms > 0
            && event.summary.contains("created")
    }));
    assert!(events.iter().any(|event| {
        event.kind == projector_domain::ProvenanceEventKind::DocumentUpdated
            && event.mount_relative_path.as_deref() == Some("private")
            && event.relative_path.as_deref() == Some("briefs/provenance.html")
            && event.summary.contains("updated")
    }));
    assert!(events.iter().any(|event| {
        event.kind == projector_domain::ProvenanceEventKind::DocumentMoved
            && event.mount_relative_path.as_deref() == Some("notes")
            && event.relative_path.as_deref() == Some("archive/provenance.html")
            && event.summary.contains("moved")
    }));
    assert!(events.iter().any(|event| {
        event.kind == projector_domain::ProvenanceEventKind::DocumentDeleted
            && event.summary.contains("deleted")
    }));
}

// @verifies PROJECTOR.HISTORY.DOCUMENT_BODY_HISTORY
#[test]
fn server_state_retains_document_body_revisions() {
    let repo = temp_repo("body-history");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    let first_sync = run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    let workspace_id = first_sync
        .lines()
        .find_map(|line| line.strip_prefix("workspace_id: "))
        .expect("workspace id")
        .to_owned();

    fs::create_dir_all(repo.join("private/briefs")).expect("create local subdir");
    fs::write(
        repo.join("private/briefs/history.html"),
        "<p>created revision</p>\n",
    )
    .expect("write create");
    run_projector(&repo, &["sync"]);

    fs::write(
        repo.join("private/briefs/history.html"),
        "<p>updated revision</p>\n",
    )
    .expect("write update");
    run_projector(&repo, &["sync"]);

    let body_history = fs::read_to_string(
        state_dir
            .join("workspaces")
            .join(&workspace_id)
            .join("body_revisions.json"),
    )
    .expect("read body history");
    let body_history: serde_json::Value =
        serde_json::from_str(&body_history).expect("parse body history");
    let revisions = body_history.as_array().expect("body revisions array");

    assert_eq!(revisions.len(), 2);
    assert_eq!(revisions[0]["base_text"], "");
    assert_eq!(revisions[0]["conflicted"], false);
    assert_eq!(revisions[1]["base_text"], "<p>created revision</p>\n");
    assert_eq!(revisions[1]["conflicted"], false);
    assert!(
        revisions[0]["body_text"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );
    assert!(
        revisions[1]["body_text"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );
}

// @verifies PROJECTOR.SERVER.HISTORY.LISTS_DOCUMENT_BODY_REVISIONS
#[test]
fn server_lists_document_body_revisions() {
    let repo = temp_repo("body-history-list");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    let first_sync = run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    let workspace_id = first_sync
        .lines()
        .find_map(|line| line.strip_prefix("workspace_id: "))
        .expect("workspace id")
        .to_owned();

    fs::create_dir_all(repo.join("private/briefs")).expect("create local subdir");
    fs::write(
        repo.join("private/briefs/history-list.html"),
        "<p>created revision</p>\n",
    )
    .expect("write create");
    run_projector(&repo, &["sync"]);

    let binding = load_workspace_binding_from_sync_config(&repo);
    let mut transport = HttpTransport::new(format!("http://{addr}"));
    let (snapshot, _) = transport.bootstrap(&binding).expect("bootstrap");
    let document_id = snapshot
        .manifest
        .entries
        .iter()
        .find(|entry| {
            !entry.deleted
                && entry.mount_relative_path == Path::new("private")
                && entry.relative_path == Path::new("briefs/history-list.html")
        })
        .expect("created entry")
        .document_id
        .as_str()
        .to_owned();

    fs::write(
        repo.join("private/briefs/history-list.html"),
        "<p>updated revision</p>\n",
    )
    .expect("write update");
    run_projector(&repo, &["sync"]);

    let revisions = list_body_revisions(&addr, &workspace_id, &document_id, 10);
    assert_eq!(revisions.len(), 2);
    assert_eq!(revisions[0].history_kind, "yrs_text_checkpoint_v1");
    assert_eq!(revisions[0].checkpoint_anchor_seq, Some(1));
    assert_eq!(revisions[0].base_text, "");
    assert_eq!(revisions[0].body_text, "<p>created revision</p>\n");
    assert_eq!(revisions[1].history_kind, "yrs_text_update_v1");
    assert_eq!(revisions[1].checkpoint_anchor_seq, Some(1));
    assert_eq!(revisions[1].base_text, "<p>created revision</p>\n");
    assert_eq!(revisions[1].body_text, "<p>updated revision</p>\n");
    assert!(!revisions[1].conflicted);
}

// @verifies PROJECTOR.SERVER.HISTORY.RENDERS_SNAPSHOT_DIFF_HISTORY
#[test]
fn server_lists_rendered_snapshot_diffs_for_body_revisions() {
    let repo = temp_repo("body-history-diff-list");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    let first_sync = run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    let workspace_id = first_sync
        .lines()
        .find_map(|line| line.strip_prefix("workspace_id: "))
        .expect("workspace id")
        .to_owned();

    fs::create_dir_all(repo.join("private/briefs")).expect("create local subdir");
    fs::write(
        repo.join("private/briefs/history-diff-list.html"),
        "<p>created revision</p>\n",
    )
    .expect("write create");
    run_projector(&repo, &["sync"]);

    fs::write(
        repo.join("private/briefs/history-diff-list.html"),
        "<p>updated revision</p>\n",
    )
    .expect("write update");
    run_projector(&repo, &["sync"]);

    let binding = load_workspace_binding_from_sync_config(&repo);
    let mut transport = HttpTransport::new(format!("http://{addr}"));
    let (snapshot, _) = transport.bootstrap(&binding).expect("bootstrap");
    let document_id = snapshot
        .manifest
        .entries
        .iter()
        .find(|entry| {
            !entry.deleted
                && entry.mount_relative_path == Path::new("private")
                && entry.relative_path == Path::new("briefs/history-diff-list.html")
        })
        .expect("created entry")
        .document_id
        .as_str()
        .to_owned();

    let revisions = list_body_revisions(&addr, &workspace_id, &document_id, 10);
    assert_eq!(revisions.len(), 2);
    assert_eq!(revisions[0].diff_lines[0], "--- base");
    assert_eq!(revisions[0].diff_lines[1], "+++ snapshot");
    assert!(
        revisions[0]
            .diff_lines
            .iter()
            .any(|line| line == "+<p>created revision</p>")
    );
    assert!(
        revisions[1]
            .diff_lines
            .iter()
            .any(|line| line == "-<p>created revision</p>")
    );
    assert!(
        revisions[1]
            .diff_lines
            .iter()
            .any(|line| line == "+<p>updated revision</p>")
    );
}

// @verifies PROJECTOR.SERVER.HISTORY.REDACTS_RETAINED_BODY_HISTORY
#[test]
fn server_can_redact_exact_text_in_retained_document_body_history() {
    let repo = temp_repo("body-history-redact");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    let first_sync = run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    let workspace_id = first_sync
        .lines()
        .find_map(|line| line.strip_prefix("workspace_id: "))
        .expect("workspace id")
        .to_owned();

    fs::create_dir_all(repo.join("private/briefs")).expect("create local subdir");
    let secret = "SECRET-123";
    fs::write(
        repo.join("private/briefs/history-redact.html"),
        format!("<p>created {secret} revision</p>\n"),
    )
    .expect("write create");
    run_projector(&repo, &["sync"]);

    fs::write(
        repo.join("private/briefs/history-redact.html"),
        format!("<p>updated {secret} revision</p>\n"),
    )
    .expect("write update");
    run_projector(&repo, &["sync"]);

    let binding = load_workspace_binding_from_sync_config(&repo);
    let mut transport = HttpTransport::new(format!("http://{addr}"));
    let (snapshot, _) = transport.bootstrap(&binding).expect("bootstrap");
    let document_id = snapshot
        .manifest
        .entries
        .iter()
        .find(|entry| {
            !entry.deleted
                && entry.mount_relative_path == Path::new("private")
                && entry.relative_path == Path::new("briefs/history-redact.html")
        })
        .expect("created entry")
        .document_id
        .as_str()
        .to_owned();

    let revisions_before = list_body_revisions(&addr, &workspace_id, &document_id, 10);
    assert_eq!(revisions_before.len(), 2);
    assert!(revisions_before.iter().any(|revision| {
        revision.base_text.contains(secret)
            || revision.body_text.contains(secret)
            || revision.diff_lines.iter().any(|line| line.contains(secret))
    }));

    redact_body_history(
        &addr,
        &workspace_id,
        binding.actor_id.as_str(),
        &document_id,
        secret,
    );

    let revisions_after = list_body_revisions(&addr, &workspace_id, &document_id, 10);
    assert_eq!(revisions_after.len(), 2);
    assert_eq!(revisions_after[0].history_kind, "yrs_text_checkpoint_v1");
    assert_eq!(revisions_after[1].history_kind, "yrs_text_update_v1");
    assert!(revisions_after.iter().all(|revision| {
        !revision.base_text.contains(secret)
            && !revision.body_text.contains(secret)
            && !revision.diff_lines.iter().any(|line| line.contains(secret))
    }));
    assert!(revisions_after.iter().any(|revision| {
        revision.base_text.contains("[REDACTED]")
            || revision.body_text.contains("[REDACTED]")
            || revision
                .diff_lines
                .iter()
                .any(|line| line.contains("[REDACTED]"))
    }));
}

// @verifies PROJECTOR.SERVER.HISTORY.PURGES_DOCUMENT_RETAINED_BODY_HISTORY
#[test]
fn server_can_purge_retained_document_body_history_for_one_document() {
    let repo = temp_repo("body-history-purge");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    let first_sync = run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    let workspace_id = first_sync
        .lines()
        .find_map(|line| line.strip_prefix("workspace_id: "))
        .expect("workspace id")
        .to_owned();

    fs::create_dir_all(repo.join("private/briefs")).expect("create local subdir");
    fs::write(
        repo.join("private/briefs/history-purge.html"),
        "<p>created revision</p>\n",
    )
    .expect("write create");
    run_projector(&repo, &["sync"]);

    fs::write(
        repo.join("private/briefs/history-purge.html"),
        "<p>updated revision</p>\n",
    )
    .expect("write update");
    run_projector(&repo, &["sync"]);

    let binding = load_workspace_binding_from_sync_config(&repo);
    let mut transport = HttpTransport::new(format!("http://{addr}"));
    let (snapshot, _) = transport.bootstrap(&binding).expect("bootstrap");
    let document_id = snapshot
        .manifest
        .entries
        .iter()
        .find(|entry| {
            !entry.deleted
                && entry.mount_relative_path == Path::new("private")
                && entry.relative_path == Path::new("briefs/history-purge.html")
        })
        .expect("created entry")
        .document_id
        .as_str()
        .to_owned();

    let revisions_before = list_body_revisions(&addr, &workspace_id, &document_id, 10);
    assert_eq!(revisions_before.len(), 2);
    assert!(
        revisions_before
            .iter()
            .any(|revision| !revision.body_text.is_empty())
    );

    purge_body_history(
        &addr,
        &workspace_id,
        binding.actor_id.as_str(),
        &document_id,
    );

    let revisions_after = list_body_revisions(&addr, &workspace_id, &document_id, 10);
    assert_eq!(revisions_after.len(), 2);
    assert!(
        revisions_after
            .iter()
            .all(|revision| revision.base_text.is_empty() && revision.body_text.is_empty())
    );

    let (live_snapshot, _) = transport.bootstrap(&binding).expect("bootstrap after purge");
    let live_body = live_snapshot
        .bodies
        .iter()
        .find(|body| body.document_id.as_str() == document_id)
        .expect("live document body");
    assert_eq!(live_body.text, "<p>updated revision</p>\n");
}

// @verifies PROJECTOR.SERVER.HISTORY.RECORDS_DESTRUCTIVE_HISTORY_SURGERY
#[test]
fn server_records_a_non_secret_audit_event_for_history_purge() {
    let repo = temp_repo("body-history-purge-audit");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    let first_sync = run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    let workspace_id = first_sync
        .lines()
        .find_map(|line| line.strip_prefix("workspace_id: "))
        .expect("workspace id")
        .to_owned();

    fs::create_dir_all(repo.join("private/briefs")).expect("create local subdir");
    let secret = "SECRET-123";
    fs::write(
        repo.join("private/briefs/history-purge-audit.html"),
        format!("<p>{secret}</p>\n"),
    )
    .expect("write create");
    run_projector(&repo, &["sync"]);

    let binding = load_workspace_binding_from_sync_config(&repo);
    let mut transport = HttpTransport::new(format!("http://{addr}"));
    let (snapshot, _) = transport.bootstrap(&binding).expect("bootstrap");
    let document_id = snapshot
        .manifest
        .entries
        .iter()
        .find(|entry| {
            !entry.deleted
                && entry.mount_relative_path == Path::new("private")
                && entry.relative_path == Path::new("briefs/history-purge-audit.html")
        })
        .expect("created entry")
        .document_id
        .as_str()
        .to_owned();

    purge_body_history(
        &addr,
        &workspace_id,
        binding.actor_id.as_str(),
        &document_id,
    );

    let events = list_events(&addr, &workspace_id, 20);
    let purge_event = events
        .iter()
        .find(|event| event.kind == projector_domain::ProvenanceEventKind::DocumentHistoryPurged)
        .expect("document history purge event");
    assert_eq!(purge_event.mount_relative_path.as_deref(), Some("private"));
    assert_eq!(
        purge_event.relative_path.as_deref(),
        Some("briefs/history-purge-audit.html")
    );
    assert!(
        purge_event
            .summary
            .contains("purged retained body history for private/briefs/history-purge-audit.html")
    );
    assert!(!purge_event.summary.contains(secret));
}

// @verifies PROJECTOR.HISTORY.MANIFEST_PATH_HISTORY
#[test]
fn server_state_retains_document_path_history() {
    let repo = temp_repo("path-history");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    let first_sync = run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    let workspace_id = first_sync
        .lines()
        .find_map(|line| line.strip_prefix("workspace_id: "))
        .expect("workspace id")
        .to_owned();

    fs::create_dir_all(repo.join("private/briefs")).expect("create local subdir");
    fs::write(
        repo.join("private/briefs/path-history.html"),
        "<p>create then move then delete</p>\n",
    )
    .expect("write create");
    run_projector(&repo, &["sync"]);

    fs::create_dir_all(repo.join("notes/archive")).expect("create move target");
    fs::rename(
        repo.join("private/briefs/path-history.html"),
        repo.join("notes/archive/path-history.html"),
    )
    .expect("move file");
    run_projector(&repo, &["sync"]);

    fs::remove_file(repo.join("notes/archive/path-history.html")).expect("delete moved file");
    run_projector(&repo, &["sync"]);

    let path_history = fs::read_to_string(
        state_dir
            .join("workspaces")
            .join(&workspace_id)
            .join("path_history.json"),
    )
    .expect("read path history");

    assert!(path_history.contains("\"event_kind\": \"document_created\""));
    assert!(path_history.contains("\"mount_path\": \"private\""));
    assert!(path_history.contains("\"relative_path\": \"briefs/path-history.html\""));
    assert!(path_history.contains("\"event_kind\": \"document_moved\""));
    assert!(path_history.contains("\"mount_path\": \"notes\""));
    assert!(path_history.contains("\"relative_path\": \"archive/path-history.html\""));
    assert!(path_history.contains("\"event_kind\": \"document_deleted\""));
    assert!(path_history.contains("\"deleted\": true"));
}

// @verifies PROJECTOR.SERVER.HISTORY.LISTS_DOCUMENT_PATH_REVISIONS
#[test]
fn server_lists_document_path_revisions() {
    let repo = temp_repo("path-history-list");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    let first_sync = run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    let workspace_id = first_sync
        .lines()
        .find_map(|line| line.strip_prefix("workspace_id: "))
        .expect("workspace id")
        .to_owned();

    fs::create_dir_all(repo.join("private/briefs")).expect("create local subdir");
    fs::write(
        repo.join("private/briefs/path-history-list.html"),
        "<p>create then move then delete</p>\n",
    )
    .expect("write create");
    run_projector(&repo, &["sync"]);

    let binding = load_workspace_binding_from_sync_config(&repo);
    let mut transport = HttpTransport::new(format!("http://{addr}"));
    let (snapshot, _) = transport.bootstrap(&binding).expect("bootstrap");
    let document_id = snapshot
        .manifest
        .entries
        .iter()
        .find(|entry| {
            !entry.deleted
                && entry.mount_relative_path == Path::new("private")
                && entry.relative_path == Path::new("briefs/path-history-list.html")
        })
        .expect("created entry")
        .document_id
        .as_str()
        .to_owned();

    fs::create_dir_all(repo.join("notes/archive")).expect("create move target");
    fs::rename(
        repo.join("private/briefs/path-history-list.html"),
        repo.join("notes/archive/path-history-list.html"),
    )
    .expect("move file");
    run_projector(&repo, &["sync"]);

    fs::remove_file(repo.join("notes/archive/path-history-list.html")).expect("delete moved file");
    run_projector(&repo, &["sync"]);

    let revisions = list_path_revisions(&addr, &workspace_id, &document_id, 10);
    assert_eq!(revisions.len(), 3);
    assert_eq!(revisions[0].event_kind, "document_created");
    assert_eq!(revisions[0].mount_path, "private");
    assert_eq!(revisions[0].relative_path, "briefs/path-history-list.html");
    assert_eq!(revisions[1].event_kind, "document_moved");
    assert_eq!(revisions[1].mount_path, "notes");
    assert_eq!(revisions[1].relative_path, "archive/path-history-list.html");
    assert_eq!(revisions[2].event_kind, "document_deleted");
    assert!(revisions[2].deleted);
}

// @verifies PROJECTOR.SERVER.HISTORY.RECONSTRUCTS_WORKSPACE_AT_CURSOR
#[test]
fn server_reconstructs_workspace_at_cursor() {
    let repo = temp_repo("workspace-reconstruct");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    let first_sync = run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    let workspace_id = first_sync
        .lines()
        .find_map(|line| line.strip_prefix("workspace_id: "))
        .expect("workspace id")
        .to_owned();

    fs::create_dir_all(repo.join("private/briefs")).expect("create local subdir");
    fs::write(
        repo.join("private/briefs/workspace-history.html"),
        "<p>created revision</p>\n",
    )
    .expect("write create");
    run_projector(&repo, &["sync"]);

    fs::write(
        repo.join("private/briefs/workspace-history.html"),
        "<p>updated revision</p>\n",
    )
    .expect("write update");
    run_projector(&repo, &["sync"]);

    fs::create_dir_all(repo.join("notes/archive")).expect("create move target");
    fs::rename(
        repo.join("private/briefs/workspace-history.html"),
        repo.join("notes/archive/workspace-history.html"),
    )
    .expect("move file");
    run_projector(&repo, &["sync"]);

    let reconstructed = reconstruct_workspace_at_cursor(&addr, &workspace_id, 2);

    assert_eq!(reconstructed.manifest.entries.len(), 1);
    let entry = &reconstructed.manifest.entries[0];
    assert!(!entry.deleted);
    assert_eq!(entry.mount_relative_path, Path::new("private"));
    assert_eq!(
        entry.relative_path,
        Path::new("briefs/workspace-history.html")
    );
    assert_eq!(reconstructed.bodies.len(), 1);
    assert_eq!(reconstructed.bodies[0].text, "<p>updated revision</p>\n");
}

// @verifies PROJECTOR.CLI.HISTORY.RENDERS_DOCUMENT_REVISIONS
#[test]
fn history_renders_document_body_and_path_revisions_for_a_live_path() {
    let repo = temp_repo("cli-history");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);

    fs::create_dir_all(repo.join("private/briefs")).expect("create local subdir");
    fs::write(
        repo.join("private/briefs/cli-history.html"),
        "<p>created revision</p>\n",
    )
    .expect("write create");
    run_projector(&repo, &["sync"]);

    fs::write(
        repo.join("private/briefs/cli-history.html"),
        "<p>updated revision</p>\n",
    )
    .expect("write update");
    run_projector(&repo, &["sync"]);

    fs::create_dir_all(repo.join("notes/archive")).expect("create move target");
    fs::rename(
        repo.join("private/briefs/cli-history.html"),
        repo.join("notes/archive/cli-history.html"),
    )
    .expect("move file");
    run_projector(&repo, &["sync"]);

    let history = run_projector(&repo, &["history", "notes/archive/cli-history.html"]);

    assert!(history.contains("path: notes/archive/cli-history.html"));
    assert!(history.contains("document_id: doc-"));
    assert!(history.contains("body_revisions: 2"));
    assert!(history.contains("kind=yrs_text_checkpoint_v1"));
    assert!(history.contains("kind=yrs_text_update_v1"));
    assert!(history.contains("checkpoint_anchor_seq=1"));
    assert!(history.contains("snapshot_text: \"<p>created revision</p>\\n\""));
    assert!(history.contains("snapshot_text: \"<p>updated revision</p>\\n\""));
    assert!(history.contains("--- base"));
    assert!(history.contains("+++ snapshot"));
    assert!(history.contains("+<p>created revision</p>"));
    assert!(history.contains("-<p>created revision</p>"));
    assert!(history.contains("+<p>updated revision</p>"));
    assert!(history.contains("path_revisions: 2"));
    assert!(history.contains("kind=document_created"));
    assert!(history.contains("path=private/briefs/cli-history.html"));
    assert!(history.contains("kind=document_moved"));
    assert!(history.contains("path=notes/archive/cli-history.html"));
}

// @verifies PROJECTOR.HISTORY.SNAPSHOT_DIFF_HISTORY
#[test]
fn history_renders_snapshot_and_diff_over_retained_body_checkpoints() {
    let repo = temp_repo("cli-history-diff");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);

    fs::create_dir_all(repo.join("private/briefs")).expect("create local subdir");
    fs::write(
        repo.join("private/briefs/cli-history-diff.html"),
        "<p>created revision</p>\n",
    )
    .expect("write create");
    run_projector(&repo, &["sync"]);

    fs::write(
        repo.join("private/briefs/cli-history-diff.html"),
        "<p>updated revision</p>\n",
    )
    .expect("write update");
    run_projector(&repo, &["sync"]);

    let history = run_projector(&repo, &["history", "private/briefs/cli-history-diff.html"]);

    assert!(history.contains("kind=yrs_text_checkpoint_v1"));
    assert!(history.contains("kind=yrs_text_update_v1"));
    assert!(history.contains("checkpoint_anchor_seq=1"));
    assert!(history.contains("snapshot_text: \"<p>created revision</p>\\n\""));
    assert!(history.contains("snapshot_text: \"<p>updated revision</p>\\n\""));
    assert!(history.contains("--- base"));
    assert!(history.contains("+++ snapshot"));
    assert!(history.contains("+<p>created revision</p>"));
    assert!(history.contains("-<p>created revision</p>"));
    assert!(history.contains("+<p>updated revision</p>"));
}

// @verifies PROJECTOR.CLI.HISTORY.RENDERS_WORKSPACE_RECONSTRUCTION
#[test]
fn history_renders_workspace_reconstruction_for_a_cursor() {
    let repo = temp_repo("cli-history-workspace");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);

    fs::create_dir_all(repo.join("private/briefs")).expect("create local subdir");
    fs::write(
        repo.join("private/briefs/workspace-preview.html"),
        "<p>created revision</p>\n",
    )
    .expect("write create");
    run_projector(&repo, &["sync"]);

    fs::write(
        repo.join("private/briefs/workspace-preview.html"),
        "<p>updated revision</p>\n",
    )
    .expect("write update");
    run_projector(&repo, &["sync"]);

    fs::create_dir_all(repo.join("notes/archive")).expect("create move target");
    fs::rename(
        repo.join("private/briefs/workspace-preview.html"),
        repo.join("notes/archive/workspace-preview.html"),
    )
    .expect("move file");
    run_projector(&repo, &["sync"]);

    let history = run_projector(&repo, &["history", "--cursor", "2"]);

    assert!(history.contains("workspace_cursor: 2"));
    assert!(history.contains("manifest_entries: 1"));
    assert!(history.contains("manifest_entry: document_id=doc-"));
    assert!(history.contains("deleted=false path=private/briefs/workspace-preview.html"));
    assert!(history.contains("body_documents: 1"));
    assert!(history.contains("body_document: document_id=doc-"));
    assert!(history.contains("path=private/briefs/workspace-preview.html"));
    assert!(history.contains("text=\"<p>updated revision</p>\\n\""));
}

// @verifies PROJECTOR.HISTORY.RESTORABLE_WORKSPACE_STATE
#[test]
fn history_workspace_reconstruction_preserves_earlier_cursor_state() {
    let repo = temp_repo("cli-history-workspace-restorable");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);

    fs::create_dir_all(repo.join("private/briefs")).expect("create local subdir");
    fs::write(
        repo.join("private/briefs/workspace-restorable.html"),
        "<p>created revision</p>\n",
    )
    .expect("write create");
    run_projector(&repo, &["sync"]);

    fs::write(
        repo.join("private/briefs/workspace-restorable.html"),
        "<p>updated revision</p>\n",
    )
    .expect("write update");
    run_projector(&repo, &["sync"]);

    fs::create_dir_all(repo.join("notes/archive")).expect("create move target");
    fs::rename(
        repo.join("private/briefs/workspace-restorable.html"),
        repo.join("notes/archive/workspace-restorable.html"),
    )
    .expect("move file");
    run_projector(&repo, &["sync"]);

    let history = run_projector(&repo, &["history", "--cursor", "1"]);

    assert!(history.contains("workspace_cursor: 1"));
    assert!(history.contains("deleted=false path=private/briefs/workspace-restorable.html"));
    assert!(history.contains("text=\"<p>created revision</p>\\n\""));
    assert!(!history.contains("notes/archive/workspace-restorable.html"));
}

fn assert_projector_redact_rewrites_history() {
    let repo = temp_repo("cli-redact");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);

    fs::create_dir_all(repo.join("private/briefs")).expect("create local subdir");
    let secret = "SECRET-123";
    fs::write(
        repo.join("private/briefs/cli-redact.html"),
        format!("<p>created {secret} revision</p>\n"),
    )
    .expect("write create");
    run_projector(&repo, &["sync"]);

    fs::write(
        repo.join("private/briefs/cli-redact.html"),
        format!("<p>updated {secret} revision</p>\n"),
    )
    .expect("write update");
    run_projector(&repo, &["sync"]);

    let preview = run_projector(
        &repo,
        &["redact", secret, "private/briefs/cli-redact.html"],
    );
    assert!(preview.contains("path: private/briefs/cli-redact.html"));
    assert!(preview.contains("matching_revisions: 2"));
    assert!(preview.contains("replacement: [REDACTED]"));
    assert!(preview.contains("match: seq=1"));
    assert!(preview.contains("match: seq=2"));
    assert!(preview.contains(&format!("excerpt: <p>created {secret} revision</p>")));
    assert!(preview.contains("next: rerun with --confirm to apply this redaction"));

    let before = run_projector(&repo, &["history", "private/briefs/cli-redact.html"]);
    assert!(before.contains(secret));

    let applied = run_projector(
        &repo,
        &["redact", "--confirm", secret, "private/briefs/cli-redact.html"],
    );
    assert!(applied.contains("redaction: applied"));

    let after = run_projector(&repo, &["history", "private/briefs/cli-redact.html"]);
    assert!(!after.contains(secret));
    assert!(after.contains("[REDACTED]"));
}

// @verifies PROJECTOR.CLI.REDACT.PREVIEWS_AND_APPLIES_EXACT_TEXT_REWRITE
#[test]
fn projector_redact_previews_then_applies_exact_text_history_rewrite() {
    assert_projector_redact_rewrites_history();
}

// @verifies PROJECTOR.HISTORY.CONTENT_REDACTION
#[test]
fn history_content_redaction_rewrites_exact_text_by_path() {
    assert_projector_redact_rewrites_history();
}

fn assert_projector_purge_clears_retained_history_and_records_audit() {
    let repo = temp_repo("cli-purge");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    let first_sync = run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    let workspace_id = first_sync
        .lines()
        .find_map(|line| line.strip_prefix("workspace_id: "))
        .expect("workspace id")
        .to_owned();

    fs::create_dir_all(repo.join("private/briefs")).expect("create local subdir");
    fs::write(
        repo.join("private/briefs/cli-purge.html"),
        "<p>created revision</p>\n",
    )
    .expect("write create");
    run_projector(&repo, &["sync"]);

    fs::write(
        repo.join("private/briefs/cli-purge.html"),
        "<p>updated revision</p>\n",
    )
    .expect("write update");
    run_projector(&repo, &["sync"]);

    let preview = run_projector(&repo, &["purge", "private/briefs/cli-purge.html"]);
    assert!(preview.contains("path: private/briefs/cli-purge.html"));
    assert!(preview.contains("retained_revisions: 2"));
    assert!(preview.contains("clearable_revisions: 2"));
    assert!(preview.contains("revision: seq=1 kind=yrs_text_checkpoint_v1"));
    assert!(preview.contains("revision: seq=2 kind=yrs_text_update_v1"));
    assert!(preview.contains("next: rerun with --confirm to purge retained history"));

    let history_before = run_projector(&repo, &["history", "private/briefs/cli-purge.html"]);
    assert!(history_before.contains("<p>updated revision</p>"));

    let applied = run_projector(
        &repo,
        &["purge", "--confirm", "private/briefs/cli-purge.html"],
    );
    assert!(applied.contains("purge: applied"));

    let history_after = run_projector(&repo, &["history", "private/briefs/cli-purge.html"]);
    assert!(history_after.contains("body_revisions: 2"));
    assert!(history_after.contains("snapshot_text: \"\""));

    let binding = load_workspace_binding_from_sync_config(&repo);
    let mut transport = HttpTransport::new(format!("http://{addr}"));
    let events = transport.provenance(&binding, 20).expect("list provenance");
    assert!(events.iter().any(|event| {
        event.kind == projector_domain::ProvenanceEventKind::DocumentHistoryPurged
            && event.summary.contains("purged retained body history")
    }));

    let (snapshot, _) = transport.bootstrap(&binding).expect("bootstrap after purge");
    let document_id = snapshot
        .manifest
        .entries
        .iter()
        .find(|entry| {
            !entry.deleted
                && entry.mount_relative_path == Path::new("private")
                && entry.relative_path == Path::new("briefs/cli-purge.html")
        })
        .expect("created entry")
        .document_id
        .as_str()
        .to_owned();
    let revisions = list_body_revisions(&addr, &workspace_id, &document_id, 10);
    assert!(revisions
        .iter()
        .all(|revision| revision.base_text.is_empty() && revision.body_text.is_empty()));
}

// @verifies PROJECTOR.CLI.PURGE.PREVIEWS_AND_APPLIES_RETAINED_HISTORY_SURGERY
#[test]
fn projector_purge_previews_then_applies_retained_history_surgery() {
    assert_projector_purge_clears_retained_history_and_records_audit();
}

// @verifies PROJECTOR.HISTORY.DOCUMENT_HISTORY_PURGE
#[test]
fn history_purge_clears_retained_history_by_path() {
    assert_projector_purge_clears_retained_history_and_records_audit();
}

// @verifies PROJECTOR.HISTORY.DESTRUCTIVE_HISTORY_AUDIT
#[test]
fn history_purge_records_non_secret_audit_trail() {
    assert_projector_purge_clears_retained_history_and_records_audit();
}

// @verifies PROJECTOR.CLI.REDACT.INTERACTIVE_CONFIRMATION
#[test]
fn projector_redact_can_apply_after_terminal_confirmation() {
    let repo = temp_repo("cli-redact-tty");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);

    fs::create_dir_all(repo.join("private/briefs")).expect("create local subdir");
    let secret = "SECRET-123";
    fs::write(
        repo.join("private/briefs/cli-redact-tty.html"),
        format!("<p>created {secret} revision</p>\n"),
    )
    .expect("write create");
    run_projector(&repo, &["sync"]);

    let output = run_projector_tty(
        &repo,
        &["redact", secret, "private/briefs/cli-redact-tty.html"],
        "y\n",
    );
    assert!(output.contains("Apply retained-history redaction? [y/N]"));
    assert!(output.contains("redaction: applied"));

    let history = run_projector(&repo, &["history", "private/briefs/cli-redact-tty.html"]);
    assert!(!history.contains(secret));
    assert!(history.contains("[REDACTED]"));
}

// @verifies PROJECTOR.CLI.PURGE.INTERACTIVE_CONFIRMATION
#[test]
fn projector_purge_can_apply_after_terminal_confirmation() {
    let repo = temp_repo("cli-purge-tty");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);

    fs::create_dir_all(repo.join("private/briefs")).expect("create local subdir");
    fs::write(
        repo.join("private/briefs/cli-purge-tty.html"),
        "<p>created revision</p>\n",
    )
    .expect("write create");
    run_projector(&repo, &["sync"]);

    let output = run_projector_tty(
        &repo,
        &["purge", "private/briefs/cli-purge-tty.html"],
        "y\n",
    );
    assert!(output.contains("Apply retained-history purge? [y/N]"));
    assert!(output.contains("purge: applied"));

    let history = run_projector(&repo, &["history", "private/briefs/cli-purge-tty.html"]);
    assert!(history.contains("snapshot_text: \"\""));
}

// @verifies PROJECTOR.SERVER.HISTORY.RESTORES_WORKSPACE_AT_CURSOR
#[test]
fn server_restores_workspace_at_cursor() {
    let repo = temp_repo("server-workspace-restore");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    let first_sync = run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    let workspace_id = first_sync
        .lines()
        .find_map(|line| line.strip_prefix("workspace_id: "))
        .expect("workspace id")
        .to_owned();
    let actor_id = first_sync
        .lines()
        .find_map(|line| line.strip_prefix("actor_id: "))
        .expect("actor id")
        .to_owned();

    fs::create_dir_all(repo.join("private/briefs")).expect("create local subdir");
    fs::write(
        repo.join("private/briefs/workspace-restore.html"),
        "<p>created revision</p>\n",
    )
    .expect("write create");
    run_projector(&repo, &["sync"]);

    fs::write(
        repo.join("private/briefs/workspace-restore.html"),
        "<p>updated revision</p>\n",
    )
    .expect("write update");
    run_projector(&repo, &["sync"]);

    fs::create_dir_all(repo.join("notes/archive")).expect("create move target");
    fs::rename(
        repo.join("private/briefs/workspace-restore.html"),
        repo.join("notes/archive/workspace-restore.html"),
    )
    .expect("move file");
    run_projector(&repo, &["sync"]);

    restore_workspace_at_cursor(&addr, &workspace_id, &actor_id, 3, 1);

    let binding = load_workspace_binding_from_sync_config(&repo);
    let mut transport = HttpTransport::new(format!("http://{addr}"));
    let (restored_snapshot, _) = transport.bootstrap(&binding).expect("bootstrap restored");
    let entry = restored_snapshot
        .manifest
        .entries
        .iter()
        .find(|entry| !entry.deleted)
        .expect("restored live entry");
    assert_eq!(entry.mount_relative_path, Path::new("private"));
    assert_eq!(
        entry.relative_path,
        Path::new("briefs/workspace-restore.html")
    );
    assert_eq!(restored_snapshot.bodies.len(), 1);
    assert_eq!(
        restored_snapshot.bodies[0].text,
        "<p>created revision</p>\n"
    );
}

// @verifies PROJECTOR.SERVER.HISTORY.RESTORES_DOCUMENT_BODY_REVISION
#[test]
fn server_restores_document_body_revision() {
    let repo = temp_repo("body-restore-server");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    let first_sync = run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    let workspace_id = first_sync
        .lines()
        .find_map(|line| line.strip_prefix("workspace_id: "))
        .expect("workspace id")
        .to_owned();

    fs::create_dir_all(repo.join("private/briefs")).expect("create local subdir");
    fs::write(
        repo.join("private/briefs/restore-server.html"),
        "<p>created revision</p>\n",
    )
    .expect("write create");
    run_projector(&repo, &["sync"]);

    let binding = load_workspace_binding_from_sync_config(&repo);
    let mut transport = HttpTransport::new(format!("http://{addr}"));
    let (snapshot, _) = transport.bootstrap(&binding).expect("bootstrap");
    let entry = snapshot
        .manifest
        .entries
        .iter()
        .find(|entry| {
            !entry.deleted
                && entry.mount_relative_path == Path::new("private")
                && entry.relative_path == Path::new("briefs/restore-server.html")
        })
        .expect("created entry");

    fs::write(
        repo.join("private/briefs/restore-server.html"),
        "<p>updated revision</p>\n",
    )
    .expect("write update");
    run_projector(&repo, &["sync"]);

    transport
        .restore_document_body_revision(&binding, &entry.document_id, 1, None, None)
        .expect("restore body revision");

    let (restored_snapshot, _) = transport.bootstrap(&binding).expect("bootstrap restored");
    let restored_body = restored_snapshot
        .bodies
        .iter()
        .find(|body| body.document_id == entry.document_id)
        .expect("restored body");
    assert_eq!(restored_body.text, "<p>created revision</p>\n");

    let revisions = list_body_revisions(&addr, &workspace_id, entry.document_id.as_str(), 10);
    assert_eq!(revisions.len(), 3);
    assert_eq!(revisions[2].base_text, "<p>updated revision</p>\n");
    assert_eq!(revisions[2].body_text, "<p>created revision</p>\n");
    assert!(!revisions[2].conflicted);
}

// @verifies PROJECTOR.HISTORY.DOCUMENT_RESTORE
#[test]
fn restore_opens_tty_browser_and_allows_browsing_without_applying_it() {
    let repo = temp_repo("cli-restore");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);

    fs::create_dir_all(repo.join("private/briefs")).expect("create local subdir");
    fs::write(
        repo.join("private/briefs/restore.html"),
        "<p>created revision</p>\n",
    )
    .expect("write create");
    run_projector(&repo, &["sync"]);

    fs::write(
        repo.join("private/briefs/restore.html"),
        "<p>updated revision</p>\n",
    )
    .expect("write update");
    run_projector(&repo, &["sync"]);

    let restore = run_projector_tty(&repo, &["restore", "private/briefs/restore.html"], "q");

    assert!(restore.contains("restore: cancelled"));
    assert!(restore.contains("selected_seq: 1"));
    assert_eq!(
        fs::read_to_string(repo.join("private/briefs/restore.html")).expect("read restored file"),
        "<p>updated revision</p>\n"
    );
}

// @verifies PROJECTOR.HISTORY.DOCUMENT_RESTORE_CONFIRM
#[test]
fn restore_uses_tty_browser_and_applies_selected_revision_after_interactive_confirmation() {
    let repo = temp_repo("cli-restore-confirm");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);

    fs::create_dir_all(repo.join("private/briefs")).expect("create local subdir");
    fs::write(
        repo.join("private/briefs/restore-confirm.html"),
        "<p>created revision</p>\n",
    )
    .expect("write create");
    run_projector(&repo, &["sync"]);

    fs::write(
        repo.join("private/briefs/restore-confirm.html"),
        "<p>updated revision</p>\n",
    )
    .expect("write update");
    run_projector(&repo, &["sync"]);

    let restore = run_projector_tty(
        &repo,
        &["restore", "private/briefs/restore-confirm.html"],
        "\ry",
    );

    assert!(restore.contains("path: private/briefs/restore-confirm.html"));
    assert!(restore.contains("restore_seq: 1"));
    assert!(restore.contains("applied: true"));
    assert_eq!(
        fs::read_to_string(repo.join("private/briefs/restore-confirm.html"))
            .expect("read restored file"),
        "<p>created revision</p>\n"
    );

    let history = run_projector(&repo, &["history", "private/briefs/restore-confirm.html"]);
    assert!(history.contains("body_revisions: 3"));
    assert!(history.contains("snapshot_text: \"<p>created revision</p>\\n\""));

    let log = run_projector(&repo, &["log"]);
    assert!(log.contains("kind=document_updated"));
    assert!(log.contains(
        "restored text document at private/briefs/restore-confirm.html from body revision 1"
    ));
}

// @verifies PROJECTOR.HISTORY.DOCUMENT_RESTORE_PREVIOUS
#[test]
fn restore_browser_defaults_to_previous_revision_preview() {
    let repo = temp_repo("cli-restore-previous");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);

    fs::create_dir_all(repo.join("private/briefs")).expect("create local subdir");
    fs::write(
        repo.join("private/briefs/restore-previous.html"),
        "<p>created revision</p>\n",
    )
    .expect("write create");
    run_projector(&repo, &["sync"]);

    fs::write(
        repo.join("private/briefs/restore-previous.html"),
        "<p>updated revision</p>\n",
    )
    .expect("write update");
    run_projector(&repo, &["sync"]);

    let restore = run_projector_tty(
        &repo,
        &["restore", "private/briefs/restore-previous.html"],
        "q",
    );

    assert!(restore.contains("restore: cancelled"));
    assert!(restore.contains("selected_seq: 1"));
    assert_eq!(
        fs::read_to_string(repo.join("private/briefs/restore-previous.html"))
            .expect("read restored file"),
        "<p>updated revision</p>\n"
    );
}

// @verifies PROJECTOR.HISTORY.DOCUMENT_RESTORE_SCRIPTED
#[test]
fn restore_supports_noninteractive_seq_preview_and_confirm() {
    let repo = temp_repo("cli-restore-scripted");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);

    fs::create_dir_all(repo.join("private/briefs")).expect("create local subdir");
    fs::write(
        repo.join("private/briefs/restore-scripted.html"),
        "<p>created revision</p>\n",
    )
    .expect("write create");
    run_projector(&repo, &["sync"]);

    fs::write(
        repo.join("private/briefs/restore-scripted.html"),
        "<p>updated revision</p>\n",
    )
    .expect("write update");
    run_projector(&repo, &["sync"]);

    let preview = run_projector(
        &repo,
        &[
            "restore",
            "--seq",
            "1",
            "private/briefs/restore-scripted.html",
        ],
    );
    assert!(preview.contains("restore_seq: 1"));
    assert!(preview.contains("applied: false"));
    assert_eq!(
        fs::read_to_string(repo.join("private/briefs/restore-scripted.html"))
            .expect("read preview file"),
        "<p>updated revision</p>\n"
    );

    let restore = run_projector(
        &repo,
        &[
            "restore",
            "--seq",
            "1",
            "--confirm",
            "private/briefs/restore-scripted.html",
        ],
    );
    assert!(restore.contains("restore_seq: 1"));
    assert!(restore.contains("applied: true"));
    assert_eq!(
        fs::read_to_string(repo.join("private/briefs/restore-scripted.html"))
            .expect("read restored file"),
        "<p>created revision</p>\n"
    );
}

// @verifies PROJECTOR.SERVER.HISTORY.REVIVES_DELETED_DOCUMENT_AT_LAST_PATH
#[test]
fn server_restore_revives_deleted_document_at_last_path() {
    let repo = temp_repo("server-restore-deleted");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);

    fs::create_dir_all(repo.join("private/briefs")).expect("create local subdir");
    fs::write(
        repo.join("private/briefs/restore-deleted-server.html"),
        "<p>created revision</p>\n",
    )
    .expect("write create");
    run_projector(&repo, &["sync"]);

    let binding = load_workspace_binding_from_sync_config(&repo);
    let mut transport = HttpTransport::new(format!("http://{addr}"));
    let (snapshot, _) = transport.bootstrap(&binding).expect("bootstrap");
    let entry = snapshot
        .manifest
        .entries
        .iter()
        .find(|entry| {
            !entry.deleted
                && entry.mount_relative_path == Path::new("private")
                && entry.relative_path == Path::new("briefs/restore-deleted-server.html")
        })
        .expect("created entry");

    fs::write(
        repo.join("private/briefs/restore-deleted-server.html"),
        "<p>updated revision</p>\n",
    )
    .expect("write update");
    run_projector(&repo, &["sync"]);
    fs::remove_file(repo.join("private/briefs/restore-deleted-server.html"))
        .expect("delete local file");
    run_projector(&repo, &["sync"]);

    transport
        .restore_document_body_revision(&binding, &entry.document_id, 1, None, None)
        .expect("restore deleted body revision");

    let (restored_snapshot, _) = transport.bootstrap(&binding).expect("bootstrap restored");
    let restored_entry = restored_snapshot
        .manifest
        .entries
        .iter()
        .find(|candidate| candidate.document_id == entry.document_id)
        .expect("restored entry");
    assert!(!restored_entry.deleted);
    let restored_body = restored_snapshot
        .bodies
        .iter()
        .find(|body| body.document_id == entry.document_id)
        .expect("restored body");
    assert_eq!(restored_body.text, "<p>created revision</p>\n");

    let path_revisions = list_path_revisions(
        &addr,
        binding.workspace_id.as_str(),
        entry.document_id.as_str(),
        10,
    );
    assert_eq!(
        path_revisions
            .last()
            .expect("latest path revision")
            .event_kind,
        "document_restored"
    );
}

// @verifies PROJECTOR.SERVER.HISTORY.RESOLVES_DOCUMENT_BY_HISTORICAL_PATH
#[test]
fn server_resolves_document_by_historical_moved_path() {
    let repo = temp_repo("server-resolve-historical");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    let first_sync = run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    let workspace_id = first_sync
        .lines()
        .find_map(|line| line.strip_prefix("workspace_id: "))
        .expect("workspace id")
        .to_owned();

    fs::create_dir_all(repo.join("private/briefs")).expect("create local subdir");
    fs::write(
        repo.join("private/briefs/resolve-historical.html"),
        "<p>created revision</p>\n",
    )
    .expect("write create");
    run_projector(&repo, &["sync"]);

    let binding = load_workspace_binding_from_sync_config(&repo);
    let mut transport = HttpTransport::new(format!("http://{addr}"));
    let (snapshot, _) = transport.bootstrap(&binding).expect("bootstrap");
    let entry = snapshot
        .manifest
        .entries
        .iter()
        .find(|entry| {
            !entry.deleted
                && entry.mount_relative_path == Path::new("private")
                && entry.relative_path == Path::new("briefs/resolve-historical.html")
        })
        .expect("created entry");

    fs::create_dir_all(repo.join("notes/archive")).expect("create move target");
    fs::rename(
        repo.join("private/briefs/resolve-historical.html"),
        repo.join("notes/archive/resolve-historical.html"),
    )
    .expect("move file");
    run_projector(&repo, &["sync"]);

    let resolved = resolve_document_by_historical_path(
        &addr,
        &workspace_id,
        "private",
        "briefs/resolve-historical.html",
    );
    assert_eq!(resolved, entry.document_id.as_str());
}

// @verifies PROJECTOR.HISTORY.DOCUMENT_RESTORE_REVIVES_DELETED_PATH
#[test]
fn restore_revives_deleted_document_at_last_path_after_interactive_confirmation() {
    let repo = temp_repo("cli-restore-deleted");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);

    fs::create_dir_all(repo.join("private/briefs")).expect("create local subdir");
    fs::write(
        repo.join("private/briefs/restore-deleted.html"),
        "<p>created revision</p>\n",
    )
    .expect("write create");
    run_projector(&repo, &["sync"]);

    fs::write(
        repo.join("private/briefs/restore-deleted.html"),
        "<p>updated revision</p>\n",
    )
    .expect("write update");
    run_projector(&repo, &["sync"]);

    fs::remove_file(repo.join("private/briefs/restore-deleted.html")).expect("delete local file");
    run_projector(&repo, &["sync"]);

    let restore = run_projector_tty(
        &repo,
        &["restore", "private/briefs/restore-deleted.html"],
        "\ry",
    );

    assert!(restore.contains("path: private/briefs/restore-deleted.html"));
    assert!(restore.contains("restore_seq: 1"));
    assert!(restore.contains("applied: true"));
    assert_eq!(
        fs::read_to_string(repo.join("private/briefs/restore-deleted.html"))
            .expect("read revived file"),
        "<p>created revision</p>\n"
    );

    let log = run_projector(&repo, &["log"]);
    assert!(log.contains(
        "restored text document at private/briefs/restore-deleted.html from body revision 1"
    ));
}

// @verifies PROJECTOR.HISTORY.DOCUMENT_RESTORE_HISTORICAL_MOVED_PATH
#[test]
fn restore_moves_document_back_to_historical_path_after_interactive_confirmation() {
    let repo = temp_repo("cli-restore-historical-move");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);

    fs::create_dir_all(repo.join("private/briefs")).expect("create local subdir");
    fs::write(
        repo.join("private/briefs/restore-historical.html"),
        "<p>created revision</p>\n",
    )
    .expect("write create");
    run_projector(&repo, &["sync"]);

    fs::write(
        repo.join("private/briefs/restore-historical.html"),
        "<p>updated revision</p>\n",
    )
    .expect("write update");
    run_projector(&repo, &["sync"]);

    fs::create_dir_all(repo.join("notes/archive")).expect("create move target");
    fs::rename(
        repo.join("private/briefs/restore-historical.html"),
        repo.join("notes/archive/restore-historical.html"),
    )
    .expect("move file");
    run_projector(&repo, &["sync"]);

    let restore = run_projector_tty(
        &repo,
        &["restore", "private/briefs/restore-historical.html"],
        "\ry",
    );

    assert!(restore.contains("path: private/briefs/restore-historical.html"));
    assert!(restore.contains("restore_seq: 1"));
    assert!(restore.contains("applied: true"));
    assert_eq!(
        fs::read_to_string(repo.join("private/briefs/restore-historical.html"))
            .expect("read restored historical file"),
        "<p>created revision</p>\n"
    );
    assert!(!repo.join("notes/archive/restore-historical.html").exists());

    let history = run_projector(
        &repo,
        &["history", "private/briefs/restore-historical.html"],
    );
    assert!(history.contains("path_revisions: 3"));
    assert!(history.contains("kind=document_restored"));
}

// @verifies PROJECTOR.SYNC.TEXT_CONVERGENCE
#[test]
fn sync_converges_concurrent_text_updates_across_two_bound_checkouts() {
    let repo_a = temp_repo("text-convergence-a");
    let repo_b = temp_repo("text-convergence-b");
    fs::write(repo_a.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    fs::write(repo_b.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");

    let state_dir = repo_a.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    run_projector(&repo_a, &["sync", "--server", &addr, "private", "notes"]);
    clone_sync_config_for_repo(&repo_a, &repo_b, "actor-text-convergence-b");

    fs::create_dir_all(repo_a.join("private/briefs")).expect("create base dir");
    fs::write(
        repo_a.join("private/briefs/converge.html"),
        "<p>shared base</p>\n",
    )
    .expect("write base file");
    run_projector(&repo_a, &["sync"]);
    run_projector(&repo_b, &["sync"]);

    fs::write(
        repo_a.join("private/briefs/converge.html"),
        "<p>repo a edit</p>\n",
    )
    .expect("write repo a edit");
    fs::write(
        repo_b.join("private/briefs/converge.html"),
        "<p>repo b edit</p>\n",
    )
    .expect("write repo b edit");

    run_projector(&repo_a, &["sync"]);
    run_projector(&repo_b, &["sync"]);
    run_projector(&repo_a, &["sync"]);

    let merged_a =
        fs::read_to_string(repo_a.join("private/briefs/converge.html")).expect("read merged a");
    let merged_b =
        fs::read_to_string(repo_b.join("private/briefs/converge.html")).expect("read merged b");

    assert_eq!(merged_a, merged_b);
    assert!(!merged_a.contains("<<<<<<< existing"));
    assert!(merged_a.contains("repo a edit"));
    assert!(merged_a.contains("repo b edit"));
}
