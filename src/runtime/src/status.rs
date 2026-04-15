/**
@module PROJECTOR.RUNTIME.STATUS
Persists and loads repo-local runtime status for CLI status output and sync-loop coordination.
*/
// @fileimplements PROJECTOR.RUNTIME.STATUS
use projector_domain::WorkspaceId;
use std::fs;
use std::io;
use std::path::PathBuf;

use crate::SyncIssueDisposition;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RuntimeStatus {
    pub workspace_id: Option<WorkspaceId>,
    pub daemon_running: bool,
    pub pending_local_changes: usize,
    pub last_sync_timestamp_ms: Option<u128>,
    pub recovery_attempt_count: usize,
    pub last_recovery_action: Option<String>,
    pub sync_issue_count: usize,
    pub last_sync_issue_code: Option<String>,
    pub last_sync_issue_disposition: Option<SyncIssueDisposition>,
    pub last_sync_issue: Option<String>,
}

#[derive(Clone, Debug)]
pub struct FileRuntimeStatusStore {
    path: PathBuf,
}

impl FileRuntimeStatusStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn load(&self) -> Result<RuntimeStatus, io::Error> {
        if !self.path.exists() {
            return Ok(RuntimeStatus::default());
        }
        let content = fs::read_to_string(&self.path)?;
        let mut status = RuntimeStatus::default();

        for line in content.lines() {
            if let Some(value) = line.strip_prefix("workspace_id=") {
                status.workspace_id = Some(WorkspaceId::new(value.to_owned()));
            } else if let Some(value) = line.strip_prefix("daemon_running=") {
                status.daemon_running = value == "true";
            } else if let Some(value) = line.strip_prefix("pending_local_changes=") {
                status.pending_local_changes = value.parse::<usize>().unwrap_or_default();
            } else if let Some(value) = line.strip_prefix("last_sync_timestamp_ms=") {
                status.last_sync_timestamp_ms = value.parse::<u128>().ok();
            } else if let Some(value) = line.strip_prefix("recovery_attempt_count=") {
                status.recovery_attempt_count = value.parse::<usize>().unwrap_or_default();
            } else if let Some(value) = line.strip_prefix("last_recovery_action=") {
                status.last_recovery_action = Some(value.to_owned());
            } else if let Some(value) = line.strip_prefix("sync_issue_count=") {
                status.sync_issue_count = value.parse::<usize>().unwrap_or_default();
            } else if let Some(value) = line.strip_prefix("last_sync_issue_code=") {
                status.last_sync_issue_code = Some(value.to_owned());
            } else if let Some(value) = line.strip_prefix("last_sync_issue_disposition=") {
                status.last_sync_issue_disposition = SyncIssueDisposition::parse(value);
            } else if let Some(value) = line.strip_prefix("last_sync_issue=") {
                status.last_sync_issue = Some(value.to_owned());
            }
        }

        Ok(status)
    }

    pub fn save(&self, status: &RuntimeStatus) -> Result<(), io::Error> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut content = String::new();
        if let Some(workspace_id) = &status.workspace_id {
            content.push_str(&format!("workspace_id={}\n", workspace_id.as_str()));
        }
        content.push_str(&format!("daemon_running={}\n", status.daemon_running));
        content.push_str(&format!(
            "pending_local_changes={}\n",
            status.pending_local_changes
        ));
        if let Some(last_sync_timestamp_ms) = status.last_sync_timestamp_ms {
            content.push_str(&format!(
                "last_sync_timestamp_ms={}\n",
                last_sync_timestamp_ms
            ));
        }
        content.push_str(&format!(
            "recovery_attempt_count={}\n",
            status.recovery_attempt_count
        ));
        if let Some(last_recovery_action) = &status.last_recovery_action {
            content.push_str(&format!("last_recovery_action={last_recovery_action}\n"));
        }
        content.push_str(&format!("sync_issue_count={}\n", status.sync_issue_count));
        if let Some(last_sync_issue_code) = &status.last_sync_issue_code {
            content.push_str(&format!("last_sync_issue_code={last_sync_issue_code}\n"));
        }
        if let Some(last_sync_issue_disposition) = &status.last_sync_issue_disposition {
            content.push_str(&format!(
                "last_sync_issue_disposition={}\n",
                last_sync_issue_disposition.as_str()
            ));
        }
        if let Some(last_sync_issue) = &status.last_sync_issue {
            content.push_str(&format!(
                "last_sync_issue={}\n",
                last_sync_issue.replace('\n', " ")
            ));
        }
        fs::write(&self.path, content)
    }
}
