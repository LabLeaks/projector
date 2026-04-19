/**
@module PROJECTOR.TESTS.HISTORY_SURGERY_CLI
CLI redact and purge proof, including TTY confirmation/browser flows, under local bootstrap.
*/
// @fileimplements PROJECTOR.TESTS.HISTORY_SURGERY_CLI
use super::*;

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

    let preview = run_projector(&repo, &["redact", secret, "private/briefs/cli-redact.html"]);
    assert!(preview.contains("path: private/briefs/cli-redact.html"));
    assert!(preview.contains("matching_revisions: 2"));
    assert!(preview.contains("replacement: [REDACTED]"));
    assert!(preview.contains("match: seq=1"));
    assert!(preview.contains("match: seq=2"));
    assert!(preview.contains(&format!("excerpt: - <p>created {secret} revision</p>")));
    assert!(preview.contains("excerpt: + <p>created [REDACTED] revision</p>"));
    assert!(preview.contains("next: rerun with --confirm to apply this redaction"));

    let before = run_projector(&repo, &["history", "private/briefs/cli-redact.html"]);
    assert!(before.contains(secret));

    let applied = run_projector(
        &repo,
        &[
            "redact",
            "--confirm",
            secret,
            "private/briefs/cli-redact.html",
        ],
    );
    assert!(applied.contains("redaction: applied"));

    let after = run_projector(&repo, &["history", "private/briefs/cli-redact.html"]);
    assert!(!after.contains(secret));
    assert!(after.contains("[REDACTED]"));

    let binding = load_workspace_binding_from_sync_config(&repo);
    let mut transport = HttpTransport::new(format!("http://{addr}"));
    let events = transport.provenance(&binding, 20).expect("list provenance");
    assert!(events.iter().any(|event| {
        event.kind == projector_domain::ProvenanceEventKind::DocumentHistoryRedacted
            && event.summary.contains("redacted retained body history")
            && !event.summary.contains(secret)
    }));
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

// @verifies PROJECTOR.HISTORY.DESTRUCTIVE_HISTORY_AUDIT
#[test]
fn history_redaction_records_non_secret_audit_trail() {
    history_content_redaction_rewrites_exact_text_by_path();
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
    assert!(preview.contains("clearable_revisions: 2"));
    assert!(preview.contains("revision: seq=1 kind=yrs_text_checkpoint_v1"));
    assert!(preview.contains("revision: seq=2 kind=yrs_text_checkpoint_v1"));
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

    let (snapshot, _) = transport
        .bootstrap(&binding)
        .expect("bootstrap after purge");
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
    assert!(
        revisions
            .iter()
            .all(|revision| revision.base_text.is_empty() && revision.body_text.is_empty())
    );
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
        "\ry",
    );
    assert!(output.contains("matching_revisions: 1"));
    assert!(output.contains("selected_seq: 1"));
    assert!(output.contains("redaction: applied"));

    let history = run_projector(&repo, &["history", "private/briefs/cli-redact-tty.html"]);
    assert!(!history.contains(secret));
    assert!(history.contains("[REDACTED]"));
}

// @verifies PROJECTOR.CLI.REDACT.BROWSES_MATCHING_REVISIONS
#[test]
fn projector_redact_uses_tty_browser_to_preview_matching_revisions() {
    let repo = temp_repo("cli-redact-browser");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);

    fs::create_dir_all(repo.join("private/briefs")).expect("create local subdir");
    let secret = "SECRET-123";
    fs::write(
        repo.join("private/briefs/cli-redact-browser.html"),
        format!("<p>created {secret} revision</p>\n"),
    )
    .expect("write create");
    run_projector(&repo, &["sync"]);

    fs::write(
        repo.join("private/briefs/cli-redact-browser.html"),
        format!("<p>updated {secret} revision</p>\n"),
    )
    .expect("write update");
    run_projector(&repo, &["sync"]);

    let output = run_projector_tty(
        &repo,
        &["redact", secret, "private/briefs/cli-redact-browser.html"],
        "q",
    );
    assert!(output.contains("matching_revisions: 2"));
    assert!(output.contains("selected_seq: 1"));
    assert!(output.contains("redaction: cancelled"));

    let history = run_projector(
        &repo,
        &["history", "private/briefs/cli-redact-browser.html"],
    );
    assert!(history.contains(secret));
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
        "\ry",
    );
    assert!(output.contains("clearable_revisions: 1"));
    assert!(output.contains("selected_seq: 1"));
    assert!(output.contains("purge: applied"));

    let history = run_projector(&repo, &["history", "private/briefs/cli-purge-tty.html"]);
    assert!(history.contains("snapshot_text: \"\""));
}

// @verifies PROJECTOR.CLI.PURGE.BROWSES_CLEARABLE_REVISIONS
#[test]
fn projector_purge_uses_tty_browser_to_preview_clearable_revisions() {
    let repo = temp_repo("cli-purge-browser");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);

    fs::create_dir_all(repo.join("private/briefs")).expect("create local subdir");
    fs::write(
        repo.join("private/briefs/cli-purge-browser.html"),
        "<p>created revision</p>\n",
    )
    .expect("write create");
    run_projector(&repo, &["sync"]);

    fs::write(
        repo.join("private/briefs/cli-purge-browser.html"),
        "<p>updated revision</p>\n",
    )
    .expect("write update");
    run_projector(&repo, &["sync"]);

    let output = run_projector_tty(
        &repo,
        &["purge", "private/briefs/cli-purge-browser.html"],
        "q",
    );
    assert!(output.contains("clearable_revisions: 2"));
    assert!(output.contains("selected_seq: 1"));
    assert!(output.contains("purge: cancelled"));

    let history = run_projector(&repo, &["history", "private/briefs/cli-purge-browser.html"]);
    assert!(history.contains("updated revision"));
}
