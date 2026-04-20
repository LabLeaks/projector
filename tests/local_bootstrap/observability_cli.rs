/**
@module PROJECTOR.TESTS.OBSERVABILITY_CLI
Status, log, and daemon-management proof for local runtime behavior under local bootstrap.
*/
// @fileimplements PROJECTOR.TESTS.OBSERVABILITY_CLI
use super::*;

// @verifies PROJECTOR.CLI.HELP.RENDERS_TOP_LEVEL_USAGE
#[test]
fn top_level_help_renders_usage() {
    let repo = temp_repo("top-level-help");

    let long_help = run_projector(&repo, &["--help"]);
    assert!(long_help.contains("Usage: projector <"));
    assert!(long_help.contains("|compact <repo-relative-path>"));

    let help_command = run_projector(&repo, &["help"]);
    assert_eq!(help_command, long_help);

    let short_help = run_projector(&repo, &["-h"]);
    assert_eq!(short_help, long_help);
}

// @verifies PROJECTOR.CLI.VERSION.REPORTS_RELEASE_VERSION
#[test]
fn top_level_version_reports_release_version() {
    let repo = temp_repo("top-level-version");

    let long_version = run_projector(&repo, &["--version"]);
    assert_eq!(
        long_version.trim(),
        format!("projector {}", env!("CARGO_PKG_VERSION"))
    );

    let short_version = run_projector(&repo, &["-V"]);
    assert_eq!(short_version, long_version);
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
