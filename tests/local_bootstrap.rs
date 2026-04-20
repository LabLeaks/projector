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
#[path = "local_bootstrap/legacy_sync.rs"]
mod legacy_sync;
#[path = "local_bootstrap/observability_cli.rs"]
mod observability_cli;
#[path = "local_bootstrap/restore_cli.rs"]
mod restore_cli;
#[path = "local_bootstrap/server_api.rs"]
mod server_api;
#[path = "local_bootstrap/sync_bootstrap.rs"]
mod sync_bootstrap;
#[path = "local_bootstrap/sync_entry_cli.rs"]
mod sync_entry_cli;

use fake_transport::{
    RejectCreateTransport, RetryAfterRebootstrapTransport, RetryImmediatelyTransport,
};
use legacy_sync::run_legacy_sync_with_env;
use server_api::{
    list_body_revisions, list_events, list_path_revisions, preview_purge_body_history,
    preview_redact_body_history, purge_body_history, purge_body_history_failure,
    reconstruct_workspace_at_cursor, redact_body_history, redact_body_history_failure,
    resolve_document_by_historical_path, restore_workspace_at_cursor, seed_remote_sync_entry,
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
