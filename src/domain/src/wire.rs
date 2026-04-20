/**
@module PROJECTOR.DOMAIN.WIRE
Defines typed sync payloads for bootstrap, delta reads, document lifecycle writes, and event listing across the projector client-server boundary.
*/
// @fileimplements PROJECTOR.DOMAIN.WIRE
use serde::{Deserialize, Serialize};
use std::fmt;

use crate::{
    DocumentId, HistoryCompactionPolicy, ManifestState, ProvenanceEvent, SyncEntryKind,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BootstrapRequest {
    pub workspace_id: String,
    pub projection_relative_paths: Vec<String>,
    pub source_repo_name: Option<String>,
    pub sync_entry_kind: Option<SyncEntryKind>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BootstrapResponse {
    pub snapshot: BootstrapSnapshot,
    pub cursor: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct BootstrapSnapshot {
    pub manifest: ManifestState,
    pub bodies: Vec<DocumentBody>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DocumentBody {
    pub document_id: DocumentId,
    pub text: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CreateDocumentRequest {
    pub workspace_id: String,
    pub actor_id: String,
    pub based_on_cursor: Option<u64>,
    pub mount_relative_path: String,
    pub relative_path: String,
    pub text: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CreateDocumentResponse {
    pub document_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct UpdateDocumentRequest {
    pub workspace_id: String,
    pub actor_id: String,
    pub document_id: String,
    pub base_text: String,
    pub text: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeleteDocumentRequest {
    pub workspace_id: String,
    pub actor_id: String,
    pub based_on_cursor: Option<u64>,
    pub document_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MoveDocumentRequest {
    pub workspace_id: String,
    pub actor_id: String,
    pub based_on_cursor: Option<u64>,
    pub document_id: String,
    pub mount_relative_path: String,
    pub relative_path: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GetHistoryCompactionPolicyRequest {
    pub workspace_id: String,
    pub repo_relative_path: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryCompactionPolicySourceKind {
    Default,
    PathOverride,
    AncestorOverride,
}

impl HistoryCompactionPolicySourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::PathOverride => "path_override",
            Self::AncestorOverride => "ancestor_override",
        }
    }
}

impl std::str::FromStr for HistoryCompactionPolicySourceKind {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw {
            "default" => Ok(Self::Default),
            "path_override" => Ok(Self::PathOverride),
            "ancestor_override" => Ok(Self::AncestorOverride),
            other => Err(format!("unknown history compaction policy source kind {other}")),
        }
    }
}

impl fmt::Display for HistoryCompactionPolicySourceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GetHistoryCompactionPolicyResponse {
    pub policy: HistoryCompactionPolicy,
    pub source_kind: HistoryCompactionPolicySourceKind,
    pub source_path: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SetHistoryCompactionPolicyRequest {
    pub workspace_id: String,
    pub repo_relative_path: String,
    pub policy: HistoryCompactionPolicy,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ClearHistoryCompactionPolicyRequest {
    pub workspace_id: String,
    pub repo_relative_path: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ClearHistoryCompactionPolicyResponse {
    pub removed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ChangesSinceRequest {
    pub workspace_id: String,
    pub since_cursor: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ChangesSinceResponse {
    pub snapshot: BootstrapSnapshot,
    pub cursor: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SyncEntrySummary {
    pub sync_entry_id: String,
    pub workspace_id: String,
    pub remote_path: String,
    pub kind: SyncEntryKind,
    pub source_repo_name: Option<String>,
    pub last_updated_ms: Option<u128>,
    pub preview: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ListSyncEntriesRequest {
    pub limit: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ListSyncEntriesResponse {
    pub entries: Vec<SyncEntrySummary>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ListEventsRequest {
    pub workspace_id: String,
    pub limit: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ListEventsResponse {
    pub events: Vec<ProvenanceEvent>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentBodyHistoryKind {
    FullTextRevisionV1,
    FullTextCheckpointV1,
    YrsTextCheckpointV1,
    YrsTextUpdateV1,
}

impl DocumentBodyHistoryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FullTextRevisionV1 => "full_text_revision_v1",
            Self::FullTextCheckpointV1 => "full_text_checkpoint_v1",
            Self::YrsTextCheckpointV1 => "yrs_text_checkpoint_v1",
            Self::YrsTextUpdateV1 => "yrs_text_update_v1",
        }
    }
}

impl std::str::FromStr for DocumentBodyHistoryKind {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw {
            "full_text_revision_v1" => Ok(Self::FullTextRevisionV1),
            "full_text_checkpoint_v1" => Ok(Self::FullTextCheckpointV1),
            "yrs_text_checkpoint_v1" => Ok(Self::YrsTextCheckpointV1),
            "yrs_text_update_v1" => Ok(Self::YrsTextUpdateV1),
            other => Err(format!("unknown document body history kind {other}")),
        }
    }
}

impl fmt::Display for DocumentBodyHistoryKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DocumentBodyRevision {
    pub seq: u64,
    pub actor_id: String,
    pub document_id: String,
    pub checkpoint_anchor_seq: Option<u64>,
    pub history_kind: DocumentBodyHistoryKind,
    pub base_text: String,
    pub body_text: String,
    pub diff_lines: Vec<String>,
    pub conflicted: bool,
    pub timestamp_ms: u128,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentPathEventKind {
    DocumentCreated,
    DocumentMoved,
    DocumentDeleted,
    DocumentRestored,
    WorkspaceRestored,
}

impl DocumentPathEventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DocumentCreated => "document_created",
            Self::DocumentMoved => "document_moved",
            Self::DocumentDeleted => "document_deleted",
            Self::DocumentRestored => "document_restored",
            Self::WorkspaceRestored => "workspace_restored",
        }
    }
}

impl std::str::FromStr for DocumentPathEventKind {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw {
            "document_created" => Ok(Self::DocumentCreated),
            "document_moved" => Ok(Self::DocumentMoved),
            "document_deleted" => Ok(Self::DocumentDeleted),
            "document_restored" => Ok(Self::DocumentRestored),
            "workspace_restored" => Ok(Self::WorkspaceRestored),
            other => Err(format!("unknown document path event kind {other}")),
        }
    }
}

impl fmt::Display for DocumentPathEventKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DocumentPathRevision {
    pub seq: u64,
    pub actor_id: String,
    pub document_id: String,
    pub mount_path: String,
    pub relative_path: String,
    pub deleted: bool,
    pub event_kind: DocumentPathEventKind,
    pub timestamp_ms: u128,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ListBodyRevisionsRequest {
    pub workspace_id: String,
    pub document_id: String,
    pub limit: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ListBodyRevisionsResponse {
    pub revisions: Vec<DocumentBodyRevision>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DocumentBodyRedactionMatch {
    pub seq: u64,
    pub actor_id: String,
    pub document_id: String,
    pub checkpoint_anchor_seq: Option<u64>,
    pub history_kind: DocumentBodyHistoryKind,
    pub occurrences: usize,
    pub preview_lines: Vec<String>,
    pub timestamp_ms: u128,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PreviewRedactDocumentBodyHistoryRequest {
    pub workspace_id: String,
    pub document_id: String,
    pub exact_text: String,
    pub limit: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PreviewRedactDocumentBodyHistoryResponse {
    pub matches: Vec<DocumentBodyRedactionMatch>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DocumentBodyPurgeMatch {
    pub seq: u64,
    pub actor_id: String,
    pub document_id: String,
    pub checkpoint_anchor_seq: Option<u64>,
    pub history_kind: DocumentBodyHistoryKind,
    pub body_len: usize,
    pub timestamp_ms: u128,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PreviewPurgeDocumentBodyHistoryRequest {
    pub workspace_id: String,
    pub document_id: String,
    pub limit: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PreviewPurgeDocumentBodyHistoryResponse {
    pub matches: Vec<DocumentBodyPurgeMatch>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PurgeDocumentBodyHistoryRequest {
    pub workspace_id: String,
    pub actor_id: String,
    pub document_id: String,
    pub expected_match_seqs: Option<Vec<u64>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RedactDocumentBodyHistoryRequest {
    pub workspace_id: String,
    pub actor_id: String,
    pub document_id: String,
    pub exact_text: String,
    pub expected_match_seqs: Option<Vec<u64>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ListPathRevisionsRequest {
    pub workspace_id: String,
    pub document_id: String,
    pub limit: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ListPathRevisionsResponse {
    pub revisions: Vec<DocumentPathRevision>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResolveHistoricalPathRequest {
    pub workspace_id: String,
    pub mount_relative_path: String,
    pub relative_path: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResolveHistoricalPathResponse {
    pub document_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReconstructWorkspaceRequest {
    pub workspace_id: String,
    pub cursor: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReconstructWorkspaceResponse {
    pub snapshot: BootstrapSnapshot,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RestoreWorkspaceRequest {
    pub workspace_id: String,
    pub actor_id: String,
    pub based_on_cursor: Option<u64>,
    pub cursor: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RestoreDocumentBodyRevisionRequest {
    pub workspace_id: String,
    pub actor_id: String,
    pub document_id: String,
    pub seq: u64,
    pub target_mount_relative_path: Option<String>,
    pub target_relative_path: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ApiErrorResponse {
    pub code: String,
    pub message: String,
}
