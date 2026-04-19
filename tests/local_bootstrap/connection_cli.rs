/**
@module PROJECTOR.TESTS.CONNECTION_CLI
Connection-oriented CLI proof for connect, disconnect, deploy, and doctor flows under local bootstrap.
*/
// @fileimplements PROJECTOR.TESTS.CONNECTION_CLI
use super::*;

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
