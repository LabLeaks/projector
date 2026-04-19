/**
@module PROJECTOR.RUNTIME.TRANSPORT
Defines the runtime transport contract for bootstrap, delta reads, document lifecycle writes, and event reads, and re-exports the concrete HTTP transport implementation.
*/
// @fileimplements PROJECTOR.RUNTIME.TRANSPORT
use std::path::Path;

use projector_domain::{
    BootstrapSnapshot, DocumentBodyRedactionMatch, DocumentBodyRevision, DocumentId,
    DocumentPathRevision, ProvenanceEvent, SyncContext,
};

mod http;

pub use http::HttpTransport;

pub trait Transport {
    type Error;

    fn bootstrap(
        &mut self,
        binding: &dyn SyncContext,
    ) -> Result<(BootstrapSnapshot, u64), Self::Error>;
    fn changes_since(
        &mut self,
        binding: &dyn SyncContext,
        since_cursor: u64,
    ) -> Result<(BootstrapSnapshot, u64), Self::Error>;
    fn create_document(
        &mut self,
        binding: &dyn SyncContext,
        based_on_cursor: u64,
        mount_relative_path: &Path,
        relative_path: &Path,
        text: &str,
    ) -> Result<DocumentId, Self::Error>;
    fn update_document(
        &mut self,
        binding: &dyn SyncContext,
        document_id: &DocumentId,
        base_text: &str,
        text: &str,
    ) -> Result<(), Self::Error>;
    fn delete_document(
        &mut self,
        binding: &dyn SyncContext,
        based_on_cursor: u64,
        document_id: &DocumentId,
    ) -> Result<(), Self::Error>;
    fn move_document(
        &mut self,
        binding: &dyn SyncContext,
        based_on_cursor: u64,
        document_id: &DocumentId,
        mount_relative_path: &Path,
        relative_path: &Path,
    ) -> Result<(), Self::Error>;
    fn provenance(
        &mut self,
        binding: &dyn SyncContext,
        limit: usize,
    ) -> Result<Vec<ProvenanceEvent>, Self::Error>;
    fn list_body_revisions(
        &mut self,
        binding: &dyn SyncContext,
        document_id: &DocumentId,
        limit: usize,
    ) -> Result<Vec<DocumentBodyRevision>, Self::Error>;
    fn preview_redact_document_body_history(
        &mut self,
        binding: &dyn SyncContext,
        document_id: &DocumentId,
        exact_text: &str,
        limit: usize,
    ) -> Result<Vec<DocumentBodyRedactionMatch>, Self::Error>;
    fn list_path_revisions(
        &mut self,
        binding: &dyn SyncContext,
        document_id: &DocumentId,
        limit: usize,
    ) -> Result<Vec<DocumentPathRevision>, Self::Error>;
    fn reconstruct_workspace_at_cursor(
        &mut self,
        binding: &dyn SyncContext,
        cursor: u64,
    ) -> Result<BootstrapSnapshot, Self::Error>;
    fn restore_workspace_at_cursor(
        &mut self,
        binding: &dyn SyncContext,
        based_on_cursor: u64,
        cursor: u64,
    ) -> Result<(), Self::Error>;
    fn resolve_document_by_historical_path(
        &mut self,
        binding: &dyn SyncContext,
        mount_relative_path: &Path,
        relative_path: &Path,
    ) -> Result<DocumentId, Self::Error>;
    fn restore_document_body_revision(
        &mut self,
        binding: &dyn SyncContext,
        document_id: &DocumentId,
        seq: u64,
        target_mount_relative_path: Option<&Path>,
        target_relative_path: Option<&Path>,
    ) -> Result<(), Self::Error>;
    fn redact_document_body_history(
        &mut self,
        binding: &dyn SyncContext,
        document_id: &DocumentId,
        exact_text: &str,
        expected_match_seqs: Option<&[u64]>,
    ) -> Result<(), Self::Error>;
    fn purge_document_body_history(
        &mut self,
        binding: &dyn SyncContext,
        document_id: &DocumentId,
    ) -> Result<(), Self::Error>;
}
