/**
@module PROJECTOR.SERVER.SQLITE_HISTORY
Owns shared SQLite history row types and delegates event reads, revision reads, policy persistence, and history surgery to narrower SQLite history modules.
*/
// @fileimplements PROJECTOR.SERVER.SQLITE_HISTORY
#[path = "history_events.rs"]
mod events;
#[path = "history_policy.rs"]
mod policy;
#[path = "history_revisions.rs"]
mod revisions;
#[path = "history_surgery.rs"]
mod surgery;

pub(crate) use events::{read_events_since, read_last_event_timestamp, read_recent_events};
pub(crate) use policy::{
    clear_history_compaction_policy, enforce_history_compaction_policy,
    get_history_compaction_policy, set_history_compaction_policy,
};
pub(crate) use revisions::read_body_revisions;
pub(crate) use revisions::{
    list_body_revisions, list_path_revisions, preview_purge_document_body_history,
    preview_redact_document_body_history, read_path_history, resolve_document_by_historical_path,
};
pub(crate) use surgery::{purge_document_body_history, redact_document_body_history};
