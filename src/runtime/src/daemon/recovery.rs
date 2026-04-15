/**
@module PROJECTOR.RUNTIME.RECOVERY
Persists bounded retry state, runtime status, and local recovery or issue audit events for the sync loop.
*/
// @fileimplements PROJECTOR.RUNTIME.RECOVERY
use std::error::Error;
use std::time::{SystemTime, UNIX_EPOCH};

use projector_domain::{ProvenanceEventKind, SyncContext};

use super::SyncRunner;
use crate::{RuntimeStatus, StoredEvent, Transport, classify_sync_issue};

impl<C, T> SyncRunner<'_, C, T>
where
    C: SyncContext,
    T: Transport<Error = std::io::Error>,
{
    pub(super) fn save_status(
        &self,
        daemon_running: bool,
        pending_local_changes: usize,
        last_sync_timestamp_ms: Option<u128>,
        recovery_attempt_count: usize,
        last_recovery_action: Option<String>,
        sync_issue_count: usize,
        last_sync_issue: Option<String>,
    ) -> Result<(), std::io::Error> {
        let previous = self.status_store.load().unwrap_or_default();
        self.status_store.save(&RuntimeStatus {
            workspace_id: Some(self.binding.workspace_id().clone()),
            daemon_running,
            pending_local_changes,
            last_sync_timestamp_ms,
            recovery_attempt_count: previous.recovery_attempt_count.max(recovery_attempt_count),
            last_recovery_action: last_recovery_action.or(previous.last_recovery_action),
            sync_issue_count,
            last_sync_issue_code: None,
            last_sync_issue_disposition: None,
            last_sync_issue,
        })
    }

    pub(super) fn reset_run_status(&self) -> Result<(), Box<dyn Error>> {
        let previous = self.status_store.load().unwrap_or_default();
        self.status_store.save(&RuntimeStatus {
            workspace_id: Some(self.binding.workspace_id().clone()),
            daemon_running: false,
            pending_local_changes: 0,
            last_sync_timestamp_ms: previous.last_sync_timestamp_ms,
            recovery_attempt_count: 0,
            last_recovery_action: None,
            sync_issue_count: 0,
            last_sync_issue_code: None,
            last_sync_issue_disposition: None,
            last_sync_issue: None,
        })?;
        Ok(())
    }

    pub(super) fn persist_recovery_status(
        &self,
        recovery_attempt_count: usize,
        last_recovery_action: Option<String>,
    ) -> Result<(), Box<dyn Error>> {
        let mut status = self.status_store.load().unwrap_or_default();
        status.workspace_id = Some(self.binding.workspace_id().clone());
        status.recovery_attempt_count = recovery_attempt_count;
        status.last_recovery_action = last_recovery_action;
        self.status_store.save(&status)?;
        Ok(())
    }

    pub(super) fn record_sync_issue(
        &self,
        err: &dyn Error,
        recovery_attempt_count: usize,
        last_recovery_action: Option<String>,
    ) -> Result<(), Box<dyn Error>> {
        let issue = classify_sync_issue(err);
        let previous_status = self.status_store.load().unwrap_or_default();
        self.status_store.save(&RuntimeStatus {
            workspace_id: Some(self.binding.workspace_id().clone()),
            daemon_running: false,
            pending_local_changes: 0,
            last_sync_timestamp_ms: previous_status.last_sync_timestamp_ms,
            recovery_attempt_count,
            last_recovery_action,
            sync_issue_count: 1,
            last_sync_issue_code: Some(issue.code.clone()),
            last_sync_issue_disposition: Some(issue.disposition.clone()),
            last_sync_issue: Some(issue.message.clone()),
        })?;
        self.log.append(&StoredEvent {
            timestamp_ms: now_ms(),
            actor_id: self.binding.actor_id().clone(),
            kind: ProvenanceEventKind::SyncIssue,
            path: "-".to_owned(),
            summary: format!(
                "disposition={} code={} {}",
                issue.disposition.as_str(),
                issue.code,
                issue.message
            ),
        })?;
        Ok(())
    }

    pub(super) fn record_recovery_action(
        &self,
        action: &str,
        attempt: usize,
    ) -> Result<(), Box<dyn Error>> {
        self.log.append(&StoredEvent {
            timestamp_ms: now_ms(),
            actor_id: self.binding.actor_id().clone(),
            kind: ProvenanceEventKind::SyncRecovery,
            path: "-".to_owned(),
            summary: format!("action={action} attempt={attempt}"),
        })?;
        Ok(())
    }
}

pub(super) fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_millis()
}
