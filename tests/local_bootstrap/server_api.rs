/**
@module PROJECTOR.TESTS.SUPPORT.LOCAL_BOOTSTRAP_SERVER_API
Raw server seeding and HTTP helper calls for local-bootstrap history and restore proofs.
*/
// @fileimplements PROJECTOR.TESTS.SUPPORT.LOCAL_BOOTSTRAP_SERVER_API
use super::*;

pub(crate) fn seed_remote_sync_entry(
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

pub(crate) fn list_body_revisions(
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

pub(crate) fn get_history_compaction_policy_raw(
    addr: &str,
    workspace_id: &str,
    repo_relative_path: &str,
) -> reqwest::blocking::Response {
    reqwest::blocking::Client::new()
        .post(format!("http://{addr}/history/compact/get"))
        .json(&GetHistoryCompactionPolicyRequest {
            workspace_id: workspace_id.to_owned(),
            repo_relative_path: repo_relative_path.to_owned(),
        })
        .send()
        .expect("send history compaction get request")
}

pub(crate) fn set_history_compaction_policy_raw(
    addr: &str,
    workspace_id: &str,
    repo_relative_path: &str,
    revisions: usize,
    frequency: usize,
) -> reqwest::blocking::Response {
    reqwest::blocking::Client::new()
        .post(format!("http://{addr}/history/compact/set"))
        .json(&SetHistoryCompactionPolicyRequest {
            workspace_id: workspace_id.to_owned(),
            repo_relative_path: repo_relative_path.to_owned(),
            policy: HistoryCompactionPolicy {
                revisions,
                frequency,
            },
        })
        .send()
        .expect("send history compaction set request")
}

pub(crate) fn preview_redact_body_history(
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

pub(crate) fn preview_purge_body_history(
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

pub(crate) fn purge_body_history(
    addr: &str,
    workspace_id: &str,
    actor_id: &str,
    document_id: &str,
) {
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

pub(crate) fn purge_body_history_failure(
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

pub(crate) fn redact_body_history(
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

pub(crate) fn redact_body_history_failure(
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

pub(crate) fn list_events(addr: &str, workspace_id: &str, limit: usize) -> Vec<ProvenanceEvent> {
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

pub(crate) fn list_path_revisions(
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

pub(crate) fn resolve_document_by_historical_path(
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

pub(crate) fn reconstruct_workspace_at_cursor(
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

pub(crate) fn restore_workspace_at_cursor(
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
