/**
@module PROJECTOR.TESTS.SYNC_ENTRY_CLI
Sync-entry CLI proof for add, get, remove, and binding configuration under local bootstrap.
*/
// @fileimplements PROJECTOR.TESTS.SYNC_ENTRY_CLI
use super::*;

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
