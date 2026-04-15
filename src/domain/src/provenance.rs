/**
@module PROJECTOR.DOMAIN.PROVENANCE
Defines durable provenance event types and cursors shared by server APIs and local audit trails.
*/
// @fileimplements PROJECTOR.DOMAIN.PROVENANCE
use crate::{ActorId, DocumentId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ProvenanceEventKind {
    SyncBootstrapped,
    SyncReusedBinding,
    SyncRecovery,
    SyncIssue,
    DocumentCreated,
    DocumentMoved,
    DocumentUpdated,
    DocumentDeleted,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProvenanceEvent {
    pub cursor: u64,
    pub timestamp_ms: u128,
    pub actor_id: ActorId,
    pub document_id: Option<DocumentId>,
    pub mount_relative_path: Option<String>,
    pub relative_path: Option<String>,
    pub summary: String,
    pub kind: ProvenanceEventKind,
}
