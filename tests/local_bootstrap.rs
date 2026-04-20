/**
@module PROJECTOR.TESTS.SUPPORT.LOCAL_BOOTSTRAP
Shared local-bootstrap harness, fake transports, and test helpers for projector integration proofs.
*/
// @fileimplements PROJECTOR.TESTS.SUPPORT.LOCAL_BOOTSTRAP
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
    ActorId, BootstrapSnapshot, CheckoutBinding, DocumentBody, DocumentBodyPurgeMatch,
    DocumentBodyRedactionMatch, DocumentBodyRevision, DocumentId, DocumentKind,
    DocumentPathRevision, HistoryCompactionPolicy, ListBodyRevisionsRequest,
    ListBodyRevisionsResponse, ListEventsRequest, ListEventsResponse, ListPathRevisionsRequest,
    ListPathRevisionsResponse, ManifestEntry, ManifestState,
    PreviewPurgeDocumentBodyHistoryRequest, PreviewPurgeDocumentBodyHistoryResponse,
    PreviewRedactDocumentBodyHistoryRequest, PreviewRedactDocumentBodyHistoryResponse,
    ProjectionRoots, ProvenanceEvent, ProvenanceEventKind, PurgeDocumentBodyHistoryRequest,
    ReconstructWorkspaceRequest, ReconstructWorkspaceResponse, RedactDocumentBodyHistoryRequest,
    RepoSyncConfig, RepoSyncEntry, ResolveHistoricalPathRequest, ResolveHistoricalPathResponse,
    RestoreWorkspaceRequest, SyncContext, SyncEntryKind, WorkspaceId,
};
use projector_runtime::{
    BindingStore, FileBindingStore, FileMachineSyncRegistryStore, FileProvenanceLog,
    FileRepoSyncConfigStore, FileRuntimeStatusStore, FileServerProfileStore, HttpTransport,
    ProjectorHome, RuntimeStatus, StoredEvent, SyncIssueDisposition, SyncLoopOptions, SyncRunner,
    Transport, derive_sync_targets,
};

#[path = "local_bootstrap/compact_cli.rs"]
mod compact_cli;
#[path = "local_bootstrap/connection_cli.rs"]
mod connection_cli;
#[path = "local_bootstrap/fake_transport.rs"]
mod fake_transport;
#[path = "local_bootstrap/history_cli.rs"]
mod history_cli;
#[path = "local_bootstrap/history_server.rs"]
mod history_server;
#[path = "local_bootstrap/history_surgery.rs"]
mod history_surgery;
#[path = "local_bootstrap/observability_cli.rs"]
mod observability_cli;
#[path = "local_bootstrap/restore_cli.rs"]
mod restore_cli;
#[path = "local_bootstrap/sync_bootstrap.rs"]
mod sync_bootstrap;
#[path = "local_bootstrap/sync_entry_cli.rs"]
mod sync_entry_cli;

use fake_transport::{
    RejectCreateTransport, RetryAfterRebootstrapTransport, RetryImmediatelyTransport,
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

fn preview_redact_body_history(
    addr: &str,
    workspace_id: &str,
    document_id: &str,
    exact_text: &str,
    limit: usize,
) -> Vec<DocumentBodyRedactionMatch> {
    reqwest::blocking::Client::new()
        .post(format!("http://{addr}/history/body/redact/preview"))
        .json(&PreviewRedactDocumentBodyHistoryRequest {
            workspace_id: workspace_id.to_owned(),
            document_id: document_id.to_owned(),
            exact_text: exact_text.to_owned(),
            limit,
        })
        .send()
        .expect("send body history redact preview request")
        .error_for_status()
        .expect("body history redact preview response status")
        .json::<PreviewRedactDocumentBodyHistoryResponse>()
        .expect("decode body history redact preview response")
        .matches
}

fn preview_purge_body_history(
    addr: &str,
    workspace_id: &str,
    document_id: &str,
    limit: usize,
) -> Vec<DocumentBodyPurgeMatch> {
    reqwest::blocking::Client::new()
        .post(format!("http://{addr}/history/body/purge/preview"))
        .json(&PreviewPurgeDocumentBodyHistoryRequest {
            workspace_id: workspace_id.to_owned(),
            document_id: document_id.to_owned(),
            limit,
        })
        .send()
        .expect("send body history purge preview request")
        .error_for_status()
        .expect("body history purge preview response status")
        .json::<PreviewPurgeDocumentBodyHistoryResponse>()
        .expect("decode body history purge preview response")
        .matches
}

fn purge_body_history(addr: &str, workspace_id: &str, actor_id: &str, document_id: &str) {
    reqwest::blocking::Client::new()
        .post(format!("http://{addr}/history/body/purge"))
        .json(&PurgeDocumentBodyHistoryRequest {
            workspace_id: workspace_id.to_owned(),
            actor_id: actor_id.to_owned(),
            document_id: document_id.to_owned(),
            expected_match_seqs: None,
        })
        .send()
        .expect("send body history purge request")
        .error_for_status()
        .expect("body history purge response status");
}

fn purge_body_history_failure(
    addr: &str,
    workspace_id: &str,
    actor_id: &str,
    document_id: &str,
    expected_match_seqs: Option<&[u64]>,
) -> String {
    let response = reqwest::blocking::Client::new()
        .post(format!("http://{addr}/history/body/purge"))
        .json(&PurgeDocumentBodyHistoryRequest {
            workspace_id: workspace_id.to_owned(),
            actor_id: actor_id.to_owned(),
            document_id: document_id.to_owned(),
            expected_match_seqs: expected_match_seqs.map(|seqs| seqs.to_vec()),
        })
        .send()
        .expect("send body history purge failure request");
    assert!(
        !response.status().is_success(),
        "body history purge unexpectedly succeeded"
    );
    response.text().expect("decode purge failure body")
}

fn redact_body_history(
    addr: &str,
    workspace_id: &str,
    actor_id: &str,
    document_id: &str,
    exact_text: &str,
    expected_match_seqs: Option<&[u64]>,
) {
    reqwest::blocking::Client::new()
        .post(format!("http://{addr}/history/body/redact"))
        .json(&RedactDocumentBodyHistoryRequest {
            workspace_id: workspace_id.to_owned(),
            actor_id: actor_id.to_owned(),
            document_id: document_id.to_owned(),
            exact_text: exact_text.to_owned(),
            expected_match_seqs: expected_match_seqs.map(|seqs| seqs.to_vec()),
        })
        .send()
        .expect("send body history redact request")
        .error_for_status()
        .expect("body history redact response status");
}

fn redact_body_history_failure(
    addr: &str,
    workspace_id: &str,
    actor_id: &str,
    document_id: &str,
    exact_text: &str,
    expected_match_seqs: Option<&[u64]>,
) -> String {
    let response = reqwest::blocking::Client::new()
        .post(format!("http://{addr}/history/body/redact"))
        .json(&RedactDocumentBodyHistoryRequest {
            workspace_id: workspace_id.to_owned(),
            actor_id: actor_id.to_owned(),
            document_id: document_id.to_owned(),
            exact_text: exact_text.to_owned(),
            expected_match_seqs: expected_match_seqs.map(|seqs| seqs.to_vec()),
        })
        .send()
        .expect("send body history redact failure request");
    assert!(
        !response.status().is_success(),
        "body history redact unexpectedly succeeded"
    );
    response.text().expect("decode redact failure body")
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
