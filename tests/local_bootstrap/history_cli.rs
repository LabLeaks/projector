/**
@module PROJECTOR.TESTS.HISTORY_CLI
History CLI proof for retained revisions and workspace reconstruction under local bootstrap.
*/
// @fileimplements PROJECTOR.TESTS.HISTORY_CLI
use super::*;

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
    assert_eq!(history.matches("kind=yrs_text_checkpoint_v1").count(), 2);
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

    assert_eq!(history.matches("kind=yrs_text_checkpoint_v1").count(), 2);
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
