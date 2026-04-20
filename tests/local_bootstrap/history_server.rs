/**
@module PROJECTOR.TESTS.HISTORY_SERVER
Server-side retained-history and workspace-history proof under the local bootstrap harness.
*/
// @fileimplements PROJECTOR.TESTS.HISTORY_SERVER
use super::*;

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
    assert_eq!(
        revisions[0].history_kind,
        DocumentBodyHistoryKind::YrsTextCheckpointV1
    );
    assert_eq!(revisions[0].checkpoint_anchor_seq, Some(1));
    assert_eq!(revisions[0].base_text, "");
    assert_eq!(revisions[0].body_text, "<p>created revision</p>\n");
    assert_eq!(
        revisions[1].history_kind,
        DocumentBodyHistoryKind::YrsTextCheckpointV1
    );
    assert_eq!(revisions[1].checkpoint_anchor_seq, Some(2));
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
        None,
    );

    let revisions_after = list_body_revisions(&addr, &workspace_id, &document_id, 10);
    assert_eq!(revisions_after.len(), 2);
    assert_eq!(
        revisions_after[0].history_kind,
        DocumentBodyHistoryKind::YrsTextCheckpointV1
    );
    assert_eq!(
        revisions_after[1].history_kind,
        DocumentBodyHistoryKind::YrsTextCheckpointV1
    );
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

    let events = transport.provenance(&binding, 20).expect("list provenance");
    assert!(events.iter().any(|event| {
        event.kind == projector_domain::ProvenanceEventKind::DocumentHistoryRedacted
            && event.summary.contains("redacted retained body history")
            && !event.summary.contains(secret)
    }));
}

// @verifies PROJECTOR.SERVER.HISTORY.RECORDS_DESTRUCTIVE_HISTORY_SURGERY
#[test]
fn server_records_a_non_secret_audit_event_for_history_redaction() {
    server_can_redact_exact_text_in_retained_document_body_history();
}

// @verifies PROJECTOR.SERVER.HISTORY.PREVIEWS_REDACTION_MATCHES
#[test]
fn server_lists_retained_redaction_matches_with_preview_lines() {
    let repo = temp_repo("body-history-redact-preview");
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
        repo.join("private/briefs/history-redact-preview.html"),
        format!("<p>created {secret} revision</p>\n"),
    )
    .expect("write create");
    run_projector(&repo, &["sync"]);

    fs::write(
        repo.join("private/briefs/history-redact-preview.html"),
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
                && entry.relative_path == Path::new("briefs/history-redact-preview.html")
        })
        .expect("created entry")
        .document_id
        .as_str()
        .to_owned();

    let matches = preview_redact_body_history(&addr, &workspace_id, &document_id, secret, 10);
    assert_eq!(matches.len(), 2);
    assert_eq!(
        matches[0].history_kind,
        DocumentBodyHistoryKind::YrsTextCheckpointV1
    );
    assert_eq!(
        matches[1].history_kind,
        DocumentBodyHistoryKind::YrsTextCheckpointV1
    );
    assert!(matches.iter().all(|entry| entry.occurrences >= 1));
    assert!(matches.iter().all(|entry| {
        entry.preview_lines.iter().any(|line| line.contains(secret))
            && entry
                .preview_lines
                .iter()
                .any(|line| line.contains("[REDACTED]"))
    }));
}

// @verifies PROJECTOR.SERVER.HISTORY.REJECTS_STALE_REDACTION_PREVIEW
#[test]
fn server_redaction_rejects_stale_previewed_match_set() {
    let repo = temp_repo("body-history-redact-stale-preview");
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
        repo.join("private/briefs/history-redact-stale-preview.html"),
        format!("<p>created {secret} revision</p>\n"),
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
                && entry.relative_path == Path::new("briefs/history-redact-stale-preview.html")
        })
        .expect("created entry")
        .document_id
        .as_str()
        .to_owned();

    let matches = preview_redact_body_history(&addr, &workspace_id, &document_id, secret, 10);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].seq, 1);

    fs::write(
        repo.join("private/briefs/history-redact-stale-preview.html"),
        format!("<p>updated {secret} revision</p>\n"),
    )
    .expect("write update");
    run_projector(&repo, &["sync"]);

    let failure = redact_body_history_failure(
        &addr,
        &workspace_id,
        binding.actor_id.as_str(),
        &document_id,
        secret,
        Some(&[matches[0].seq]),
    );
    assert!(failure.contains("retained redaction preview is stale"));
    assert!(failure.contains("expected seqs [1], found [1, 2]"));

    let revisions = list_body_revisions(&addr, &workspace_id, &document_id, 10);
    assert!(
        revisions
            .iter()
            .any(|revision| revision.body_text.contains(secret))
    );
    assert!(
        revisions
            .iter()
            .all(|revision| !revision.body_text.contains("[REDACTED]"))
    );
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

    let (live_snapshot, _) = transport
        .bootstrap(&binding)
        .expect("bootstrap after purge");
    let live_body = live_snapshot
        .bodies
        .iter()
        .find(|body| body.document_id.as_str() == document_id)
        .expect("live document body");
    assert_eq!(live_body.text, "<p>updated revision</p>\n");
}

// @verifies PROJECTOR.SERVER.HISTORY.PREVIEWS_PURGE_MATCHES
#[test]
fn server_lists_retained_purge_matches() {
    let repo = temp_repo("body-history-purge-preview");
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
        repo.join("private/briefs/history-purge-preview.html"),
        "<p>created revision</p>\n",
    )
    .expect("write create");
    run_projector(&repo, &["sync"]);

    fs::write(
        repo.join("private/briefs/history-purge-preview.html"),
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
                && entry.relative_path == Path::new("briefs/history-purge-preview.html")
        })
        .expect("created entry")
        .document_id
        .as_str()
        .to_owned();

    let matches = preview_purge_body_history(&addr, &workspace_id, &document_id, 10);
    assert_eq!(matches.len(), 2);
    assert_eq!(
        matches[0].history_kind,
        DocumentBodyHistoryKind::YrsTextCheckpointV1
    );
    assert_eq!(
        matches[1].history_kind,
        DocumentBodyHistoryKind::YrsTextCheckpointV1
    );
    assert!(matches.iter().all(|entry| entry.body_len > 0));
}

// @verifies PROJECTOR.SERVER.HISTORY.REJECTS_STALE_PURGE_PREVIEW
#[test]
fn server_purge_rejects_stale_previewed_match_set() {
    let repo = temp_repo("body-history-purge-stale-preview");
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
        repo.join("private/briefs/history-purge-stale-preview.html"),
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
                && entry.relative_path == Path::new("briefs/history-purge-stale-preview.html")
        })
        .expect("created entry")
        .document_id
        .as_str()
        .to_owned();

    let matches = preview_purge_body_history(&addr, &workspace_id, &document_id, 10);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].seq, 1);

    fs::write(
        repo.join("private/briefs/history-purge-stale-preview.html"),
        "<p>updated revision</p>\n",
    )
    .expect("write update");
    run_projector(&repo, &["sync"]);

    let failure = purge_body_history_failure(
        &addr,
        &workspace_id,
        binding.actor_id.as_str(),
        &document_id,
        Some(&[matches[0].seq]),
    );
    assert!(failure.contains("retained purge preview is stale"));
    assert!(failure.contains("expected seqs [1], found [1, 2]"));
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
    assert_eq!(
        revisions[0].event_kind,
        DocumentPathEventKind::DocumentCreated
    );
    assert_eq!(revisions[0].mount_path, "private");
    assert_eq!(revisions[0].relative_path, "briefs/path-history-list.html");
    assert_eq!(
        revisions[1].event_kind,
        DocumentPathEventKind::DocumentMoved
    );
    assert_eq!(revisions[1].mount_path, "notes");
    assert_eq!(revisions[1].relative_path, "archive/path-history-list.html");
    assert_eq!(
        revisions[2].event_kind,
        DocumentPathEventKind::DocumentDeleted
    );
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
