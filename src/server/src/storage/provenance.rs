/**
@module PROJECTOR.SERVER.PROVENANCE
Owns shared workspace-provenance helpers and event-kind encoding while delegating file-backed, Postgres-backed, and synthetic provenance adapters to narrower modules.
*/
// @fileimplements PROJECTOR.SERVER.PROVENANCE
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use projector_domain::ProvenanceEventKind;

use super::StoreError;

pub(crate) use super::provenance_file::*;
pub(crate) use super::provenance_postgres::*;
pub(crate) use super::provenance_synthetic::*;

static PROVENANCE_FALLBACK_CLOCK: AtomicU64 = AtomicU64::new(1);

pub(crate) fn parse_event_kind(raw: &str) -> Result<ProvenanceEventKind, StoreError> {
    match raw {
        "document_created" => Ok(ProvenanceEventKind::DocumentCreated),
        "document_moved" => Ok(ProvenanceEventKind::DocumentMoved),
        "document_updated" => Ok(ProvenanceEventKind::DocumentUpdated),
        "document_deleted" => Ok(ProvenanceEventKind::DocumentDeleted),
        "document_history_redacted" => Ok(ProvenanceEventKind::DocumentHistoryRedacted),
        "document_history_purged" => Ok(ProvenanceEventKind::DocumentHistoryPurged),
        "sync_bootstrapped" => Ok(ProvenanceEventKind::SyncBootstrapped),
        "sync_reused_binding" => Ok(ProvenanceEventKind::SyncReusedBinding),
        "sync_recovery" => Ok(ProvenanceEventKind::SyncRecovery),
        "sync_issue" => Ok(ProvenanceEventKind::SyncIssue),
        other => Err(StoreError::new(format!("unknown event kind {other}"))),
    }
}

pub(crate) fn event_kind_db_value(kind: &ProvenanceEventKind) -> &'static str {
    match kind {
        ProvenanceEventKind::DocumentCreated => "document_created",
        ProvenanceEventKind::DocumentMoved => "document_moved",
        ProvenanceEventKind::DocumentUpdated => "document_updated",
        ProvenanceEventKind::DocumentDeleted => "document_deleted",
        ProvenanceEventKind::DocumentHistoryRedacted => "document_history_redacted",
        ProvenanceEventKind::DocumentHistoryPurged => "document_history_purged",
        ProvenanceEventKind::SyncBootstrapped => "sync_bootstrapped",
        ProvenanceEventKind::SyncReusedBinding => "sync_reused_binding",
        ProvenanceEventKind::SyncRecovery => "sync_recovery",
        ProvenanceEventKind::SyncIssue => "sync_issue",
    }
}

pub(crate) fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_else(|_| PROVENANCE_FALLBACK_CLOCK.fetch_add(1, Ordering::Relaxed) as u128)
}
