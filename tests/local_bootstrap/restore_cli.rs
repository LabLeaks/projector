/**
@module PROJECTOR.TESTS.RESTORE_CLI
Restore and historical-path proof under the local bootstrap harness.
*/
// @fileimplements PROJECTOR.TESTS.RESTORE_CLI
use super::*;

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
