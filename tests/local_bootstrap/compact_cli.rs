/**
@module PROJECTOR.TESTS.COMPACT_CLI
CLI and server compaction-policy proof under the local bootstrap harness.
*/
// @fileimplements PROJECTOR.TESTS.COMPACT_CLI
use super::*;

// @verifies PROJECTOR.CLI.COMPACT.SETS_PATH_POLICY
#[test]
fn compact_sets_path_scoped_history_policy_override() {
    let repo = temp_repo("cli-compact-set");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();
    run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);

    let output = run_projector(
        &repo,
        &[
            "compact",
            "private",
            "--revisions",
            "12",
            "--frequency",
            "4",
        ],
    );

    assert!(output.contains("compact_policy: saved"));
    assert!(output.contains("path: private"));
    assert!(output.contains("effective_policy: revisions=12 frequency=4"));
    assert!(output.contains("policy_source: path_override"));
    assert!(output.contains("source_path: private"));
}

// @verifies PROJECTOR.HISTORY.COMPACTION_POLICY
#[test]
fn compact_history_policy_persists_server_override() {
    let repo = temp_repo("compact-history-policy");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();
    run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);

    run_projector(
        &repo,
        &[
            "compact",
            "private",
            "--revisions",
            "12",
            "--frequency",
            "4",
        ],
    );

    let output = run_projector(&repo, &["compact", "private"]);
    assert!(output.contains("effective_policy: revisions=12 frequency=4"));
    assert!(output.contains("policy_source: path_override"));
    assert!(output.contains("source_path: private"));
}

// @verifies PROJECTOR.CLI.COMPACT.REPORTS_EFFECTIVE_POLICY
#[test]
fn compact_reports_default_inherited_and_exact_path_policy_sources() {
    let repo = temp_repo("cli-compact-report");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();
    run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);

    let default_output = run_projector(&repo, &["compact", "notes/today.html"]);
    assert!(default_output.contains("path: notes/today.html"));
    assert!(default_output.contains("effective_policy: revisions=100 frequency=10"));
    assert!(default_output.contains("policy_source: default"));

    run_projector(
        &repo,
        &[
            "compact",
            "private",
            "--revisions",
            "12",
            "--frequency",
            "4",
        ],
    );
    let inherited_output = run_projector(&repo, &["compact", "private/notes/today.html"]);
    assert!(inherited_output.contains("path: private/notes/today.html"));
    assert!(inherited_output.contains("effective_policy: revisions=12 frequency=4"));
    assert!(inherited_output.contains("policy_source: ancestor_override"));
    assert!(inherited_output.contains("source_path: private"));

    run_projector(
        &repo,
        &[
            "compact",
            "private/notes/today.html",
            "--revisions",
            "3",
            "--frequency",
            "2",
        ],
    );
    let exact_output = run_projector(&repo, &["compact", "private/notes/today.html"]);
    assert!(exact_output.contains("effective_policy: revisions=3 frequency=2"));
    assert!(exact_output.contains("policy_source: path_override"));
    assert!(exact_output.contains("source_path: private/notes/today.html"));
}

// @verifies PROJECTOR.HISTORY.COMPACTION_POLICY_INHERITANCE
#[test]
fn compact_history_policy_inheritance_uses_nearest_override() {
    let repo = temp_repo("compact-history-inheritance");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();
    run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);

    run_projector(
        &repo,
        &[
            "compact",
            "private",
            "--revisions",
            "12",
            "--frequency",
            "4",
        ],
    );
    run_projector(
        &repo,
        &[
            "compact",
            "private/notes/today.html",
            "--revisions",
            "3",
            "--frequency",
            "2",
        ],
    );

    let exact_output = run_projector(&repo, &["compact", "private/notes/today.html"]);
    assert!(exact_output.contains("effective_policy: revisions=3 frequency=2"));
    assert!(exact_output.contains("policy_source: path_override"));
    assert!(exact_output.contains("source_path: private/notes/today.html"));

    let inherited_output = run_projector(&repo, &["compact", "private/notes/tomorrow.html"]);
    assert!(inherited_output.contains("effective_policy: revisions=12 frequency=4"));
    assert!(inherited_output.contains("policy_source: ancestor_override"));
    assert!(inherited_output.contains("source_path: private"));
}

// @verifies PROJECTOR.CLI.COMPACT.INHERITS_PATH_POLICY
#[test]
fn compact_inherit_removes_server_override_and_falls_back_to_ancestor() {
    let repo = temp_repo("cli-compact-inherit");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();
    run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);

    run_projector(
        &repo,
        &[
            "compact",
            "private",
            "--revisions",
            "12",
            "--frequency",
            "4",
        ],
    );
    run_projector(
        &repo,
        &[
            "compact",
            "private/notes/today.html",
            "--revisions",
            "3",
            "--frequency",
            "2",
        ],
    );

    let output = run_projector(&repo, &["compact", "private/notes/today.html", "--inherit"]);
    assert!(output.contains("compact_policy: inherited"));
    assert!(output.contains("effective_policy: revisions=12 frequency=4"));
    assert!(output.contains("policy_source: ancestor_override"));
    assert!(output.contains("source_path: private"));

    let query = run_projector(&repo, &["compact", "private/notes/today.html"]);
    assert!(query.contains("effective_policy: revisions=12 frequency=4"));
    assert!(query.contains("policy_source: ancestor_override"));
}

// @verifies PROJECTOR.SERVER.HISTORY.ENFORCES_COMPACTION_POLICY
#[test]
fn server_enforces_history_compaction_policy_on_retained_body_history() {
    let repo = temp_repo("server-history-compaction");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();
    run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    run_projector(
        &repo,
        &[
            "compact",
            "private/compacted.html",
            "--revisions",
            "2",
            "--frequency",
            "2",
        ],
    );

    fs::create_dir_all(repo.join("private")).expect("create private dir");
    for (idx, body) in [
        "<p>revision one</p>\n",
        "<p>revision two</p>\n",
        "<p>revision three</p>\n",
        "<p>revision four</p>\n",
        "<p>revision five</p>\n",
        "<p>revision six</p>\n",
    ]
    .iter()
    .enumerate()
    {
        fs::write(repo.join("private/compacted.html"), body).expect("write revision");
        run_projector(&repo, &["sync"]);
        assert!(
            repo.join("private/compacted.html").exists(),
            "sync should keep materialized file after revision {}",
            idx + 1
        );
    }

    let sync_config = FileRepoSyncConfigStore::new(&repo)
        .load()
        .expect("load sync config");
    let workspace_id = sync_config
        .entries
        .first()
        .expect("sync entry exists")
        .workspace_id
        .clone();
    let document_id = resolve_document_by_historical_path(
        &addr,
        workspace_id.as_str(),
        "private",
        "compacted.html",
    );
    let revisions = list_body_revisions(&addr, workspace_id.as_str(), &document_id, 20);
    let retained_seqs = revisions
        .iter()
        .map(|revision| revision.seq)
        .collect::<Vec<_>>();
    assert!(retained_seqs.len() < 6);
    assert_eq!(retained_seqs.first().copied(), Some(1));
    assert_eq!(retained_seqs.last(), Some(&6));
    assert!(retained_seqs.contains(&5));
    assert_eq!(
        revisions[0].history_kind,
        DocumentBodyHistoryKind::YrsTextCheckpointV1
    );
    assert_eq!(
        revisions
            .last()
            .expect("latest revision retained")
            .body_text,
        "<p>revision six</p>\n"
    );
}

// @verifies PROJECTOR.SERVER.HISTORY.REJECTS_INVALID_COMPACTION_POLICY
#[test]
fn server_rejects_zero_valued_compaction_policy() {
    let repo = temp_repo("server-history-compaction-invalid-policy");
    fs::write(repo.join(".gitignore"), "private/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();
    run_projector(&repo, &["sync", "--server", &addr, "private"]);

    let sync_config = FileRepoSyncConfigStore::new(&repo)
        .load()
        .expect("load sync config");
    let workspace_id = sync_config
        .entries
        .first()
        .expect("sync entry exists")
        .workspace_id
        .clone();

    let response =
        set_history_compaction_policy_raw(&addr, workspace_id.as_str(), "private", 0, 1);
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let error = response
        .json::<ApiErrorResponse>()
        .expect("decode invalid revisions response");
    assert!(error.message.contains("revisions must be at least 1"));

    let response =
        set_history_compaction_policy_raw(&addr, workspace_id.as_str(), "private", 1, 0);
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let error = response
        .json::<ApiErrorResponse>()
        .expect("decode invalid frequency response");
    assert!(error.message.contains("frequency must be at least 1"));
}

// @verifies PROJECTOR.SERVER.HISTORY.NORMALIZES_COMPACTION_POLICY_PATHS
#[test]
fn server_normalizes_compaction_policy_paths() {
    let repo = temp_repo("server-history-compaction-normalized-path");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();
    run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);

    let sync_config = FileRepoSyncConfigStore::new(&repo)
        .load()
        .expect("load sync config");
    let workspace_id = sync_config
        .entries
        .first()
        .expect("sync entry exists")
        .workspace_id
        .clone();

    let response = set_history_compaction_policy_raw(
        &addr,
        workspace_id.as_str(),
        "./private/notes/../notes/today.html",
        7,
        3,
    );
    assert_eq!(response.status(), reqwest::StatusCode::NO_CONTENT);

    let response =
        get_history_compaction_policy_raw(&addr, workspace_id.as_str(), "private/notes/today.html");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let policy = response
        .json::<GetHistoryCompactionPolicyResponse>()
        .expect("decode compaction policy response");
    assert_eq!(policy.policy.revisions, 7);
    assert_eq!(policy.policy.frequency, 3);
    assert_eq!(
        policy.source_kind,
        HistoryCompactionPolicySourceKind::PathOverride
    );
    assert_eq!(policy.source_path.as_deref(), Some("private/notes/today.html"));
}
