/**
@module PROJECTOR.RUNTIME.HTTP_TRANSPORT
Implements the runtime transport contract over the projector HTTP API by constructing typed requests, issuing blocking HTTP calls, decoding typed responses, and mapping API errors into runtime IO errors.
*/
// @fileimplements PROJECTOR.RUNTIME.HTTP_TRANSPORT
use std::io;
use std::path::Path;

use projector_domain::{
    ApiErrorResponse, BootstrapRequest, BootstrapResponse, BootstrapSnapshot, ChangesSinceRequest,
    ChangesSinceResponse, CreateDocumentRequest, CreateDocumentResponse, DeleteDocumentRequest,
    DocumentBodyRedactionMatch, DocumentBodyRevision, DocumentId, DocumentPathRevision,
    ListBodyRevisionsRequest, ListBodyRevisionsResponse, ListEventsRequest, ListEventsResponse,
    ListPathRevisionsRequest, ListPathRevisionsResponse, ListSyncEntriesRequest,
    ListSyncEntriesResponse, MoveDocumentRequest, PreviewRedactDocumentBodyHistoryRequest,
    PreviewRedactDocumentBodyHistoryResponse, ProvenanceEvent, PurgeDocumentBodyHistoryRequest,
    ReconstructWorkspaceRequest, ReconstructWorkspaceResponse, RedactDocumentBodyHistoryRequest,
    ResolveHistoricalPathRequest, ResolveHistoricalPathResponse,
    RestoreDocumentBodyRevisionRequest, RestoreWorkspaceRequest, SyncContext, SyncEntrySummary,
    UpdateDocumentRequest,
};

use super::Transport;

#[derive(Clone, Debug)]
pub struct HttpTransport {
    base_url: String,
    client: reqwest::blocking::Client,
}

impl HttpTransport {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            client: reqwest::blocking::Client::new(),
        }
    }

    pub fn list_sync_entries(&self, limit: usize) -> Result<Vec<SyncEntrySummary>, io::Error> {
        let response = self
            .client
            .post(format!("{}/sync-entries/list", self.base_url))
            .json(&ListSyncEntriesRequest { limit })
            .send()
            .map_err(io::Error::other)?;

        if !response.status().is_success() {
            return Err(response_error("list sync entries request", response));
        }

        let payload: ListSyncEntriesResponse = response.json().map_err(io::Error::other)?;
        Ok(payload.entries)
    }
}

impl Transport for HttpTransport {
    type Error = io::Error;

    fn bootstrap(
        &mut self,
        binding: &dyn SyncContext,
    ) -> Result<(BootstrapSnapshot, u64), Self::Error> {
        let response = self
            .client
            .post(format!("{}/bootstrap", self.base_url))
            .json(&BootstrapRequest {
                workspace_id: binding.workspace_id().as_str().to_owned(),
                projection_relative_paths: binding
                    .projection_mounts()
                    .iter()
                    .map(|mount| mount.relative_path.display().to_string())
                    .collect(),
                source_repo_name: binding.source_repo_name().map(str::to_owned),
                sync_entry_kind: binding.sync_entry_kind(),
            })
            .send()
            .map_err(io::Error::other)?;

        if !response.status().is_success() {
            return Err(response_error("bootstrap request", response));
        }

        let payload: BootstrapResponse = response.json().map_err(io::Error::other)?;
        Ok((payload.snapshot, payload.cursor))
    }

    fn changes_since(
        &mut self,
        binding: &dyn SyncContext,
        since_cursor: u64,
    ) -> Result<(BootstrapSnapshot, u64), Self::Error> {
        let response = self
            .client
            .post(format!("{}/changes/since", self.base_url))
            .json(&ChangesSinceRequest {
                workspace_id: binding.workspace_id().as_str().to_owned(),
                since_cursor,
            })
            .send()
            .map_err(io::Error::other)?;

        if !response.status().is_success() {
            return Err(io::Error::other(format!(
                "changes request failed with status {}",
                response.status()
            )));
        }

        let payload: ChangesSinceResponse = response.json().map_err(io::Error::other)?;
        Ok((payload.snapshot, payload.cursor))
    }

    fn create_document(
        &mut self,
        binding: &dyn SyncContext,
        based_on_cursor: u64,
        mount_relative_path: &Path,
        relative_path: &Path,
        text: &str,
    ) -> Result<DocumentId, Self::Error> {
        let response = self
            .client
            .post(format!("{}/documents/create", self.base_url))
            .json(&CreateDocumentRequest {
                workspace_id: binding.workspace_id().as_str().to_owned(),
                actor_id: binding.actor_id().as_str().to_owned(),
                based_on_cursor: Some(based_on_cursor),
                mount_relative_path: mount_relative_path.display().to_string(),
                relative_path: relative_path.display().to_string(),
                text: text.to_owned(),
            })
            .send()
            .map_err(io::Error::other)?;

        if !response.status().is_success() {
            return Err(response_error("create document request", response));
        }

        let payload: CreateDocumentResponse = response.json().map_err(io::Error::other)?;
        Ok(DocumentId::new(payload.document_id))
    }

    fn update_document(
        &mut self,
        binding: &dyn SyncContext,
        document_id: &DocumentId,
        base_text: &str,
        text: &str,
    ) -> Result<(), Self::Error> {
        let response = self
            .client
            .post(format!("{}/documents/update", self.base_url))
            .json(&UpdateDocumentRequest {
                workspace_id: binding.workspace_id().as_str().to_owned(),
                actor_id: binding.actor_id().as_str().to_owned(),
                document_id: document_id.as_str().to_owned(),
                base_text: base_text.to_owned(),
                text: text.to_owned(),
            })
            .send()
            .map_err(io::Error::other)?;

        if !response.status().is_success() {
            return Err(io::Error::other(format!(
                "update document request failed with status {}",
                response.status()
            )));
        }

        Ok(())
    }

    fn delete_document(
        &mut self,
        binding: &dyn SyncContext,
        based_on_cursor: u64,
        document_id: &DocumentId,
    ) -> Result<(), Self::Error> {
        let response = self
            .client
            .post(format!("{}/documents/delete", self.base_url))
            .json(&DeleteDocumentRequest {
                workspace_id: binding.workspace_id().as_str().to_owned(),
                actor_id: binding.actor_id().as_str().to_owned(),
                based_on_cursor: Some(based_on_cursor),
                document_id: document_id.as_str().to_owned(),
            })
            .send()
            .map_err(io::Error::other)?;

        if !response.status().is_success() {
            return Err(response_error("delete document request", response));
        }

        Ok(())
    }

    fn move_document(
        &mut self,
        binding: &dyn SyncContext,
        based_on_cursor: u64,
        document_id: &DocumentId,
        mount_relative_path: &Path,
        relative_path: &Path,
    ) -> Result<(), Self::Error> {
        let response = self
            .client
            .post(format!("{}/documents/move", self.base_url))
            .json(&MoveDocumentRequest {
                workspace_id: binding.workspace_id().as_str().to_owned(),
                actor_id: binding.actor_id().as_str().to_owned(),
                based_on_cursor: Some(based_on_cursor),
                document_id: document_id.as_str().to_owned(),
                mount_relative_path: mount_relative_path.display().to_string(),
                relative_path: relative_path.display().to_string(),
            })
            .send()
            .map_err(io::Error::other)?;

        if !response.status().is_success() {
            return Err(response_error("move document request", response));
        }

        Ok(())
    }

    fn provenance(
        &mut self,
        binding: &dyn SyncContext,
        limit: usize,
    ) -> Result<Vec<ProvenanceEvent>, Self::Error> {
        let response = self
            .client
            .post(format!("{}/events/list", self.base_url))
            .json(&ListEventsRequest {
                workspace_id: binding.workspace_id().as_str().to_owned(),
                limit,
            })
            .send()
            .map_err(io::Error::other)?;

        if !response.status().is_success() {
            return Err(io::Error::other(format!(
                "events request failed with status {}",
                response.status()
            )));
        }

        let payload: ListEventsResponse = response.json().map_err(io::Error::other)?;
        Ok(payload.events)
    }

    fn list_body_revisions(
        &mut self,
        binding: &dyn SyncContext,
        document_id: &DocumentId,
        limit: usize,
    ) -> Result<Vec<DocumentBodyRevision>, Self::Error> {
        let response = self
            .client
            .post(format!("{}/history/body/list", self.base_url))
            .json(&ListBodyRevisionsRequest {
                workspace_id: binding.workspace_id().as_str().to_owned(),
                document_id: document_id.as_str().to_owned(),
                limit,
            })
            .send()
            .map_err(io::Error::other)?;

        if !response.status().is_success() {
            return Err(io::Error::other(format!(
                "body history request failed with status {}",
                response.status()
            )));
        }

        let payload: ListBodyRevisionsResponse = response.json().map_err(io::Error::other)?;
        Ok(payload.revisions)
    }

    fn preview_redact_document_body_history(
        &mut self,
        binding: &dyn SyncContext,
        document_id: &DocumentId,
        exact_text: &str,
        limit: usize,
    ) -> Result<Vec<DocumentBodyRedactionMatch>, Self::Error> {
        let response = self
            .client
            .post(format!("{}/history/body/redact/preview", self.base_url))
            .json(&PreviewRedactDocumentBodyHistoryRequest {
                workspace_id: binding.workspace_id().as_str().to_owned(),
                document_id: document_id.as_str().to_owned(),
                exact_text: exact_text.to_owned(),
                limit,
            })
            .send()
            .map_err(io::Error::other)?;

        if !response.status().is_success() {
            return Err(response_error(
                "preview redact body history request",
                response,
            ));
        }

        let payload: PreviewRedactDocumentBodyHistoryResponse =
            response.json().map_err(io::Error::other)?;
        Ok(payload.matches)
    }

    fn list_path_revisions(
        &mut self,
        binding: &dyn SyncContext,
        document_id: &DocumentId,
        limit: usize,
    ) -> Result<Vec<DocumentPathRevision>, Self::Error> {
        let response = self
            .client
            .post(format!("{}/history/path/list", self.base_url))
            .json(&ListPathRevisionsRequest {
                workspace_id: binding.workspace_id().as_str().to_owned(),
                document_id: document_id.as_str().to_owned(),
                limit,
            })
            .send()
            .map_err(io::Error::other)?;

        if !response.status().is_success() {
            return Err(io::Error::other(format!(
                "path history request failed with status {}",
                response.status()
            )));
        }

        let payload: ListPathRevisionsResponse = response.json().map_err(io::Error::other)?;
        Ok(payload.revisions)
    }

    fn reconstruct_workspace_at_cursor(
        &mut self,
        binding: &dyn SyncContext,
        cursor: u64,
    ) -> Result<BootstrapSnapshot, Self::Error> {
        let response = self
            .client
            .post(format!("{}/history/workspace/reconstruct", self.base_url))
            .json(&ReconstructWorkspaceRequest {
                workspace_id: binding.workspace_id().as_str().to_owned(),
                cursor,
            })
            .send()
            .map_err(io::Error::other)?;

        if !response.status().is_success() {
            return Err(response_error("reconstruct workspace request", response));
        }

        let payload: ReconstructWorkspaceResponse = response.json().map_err(io::Error::other)?;
        Ok(payload.snapshot)
    }

    fn restore_document_body_revision(
        &mut self,
        binding: &dyn SyncContext,
        document_id: &DocumentId,
        seq: u64,
        target_mount_relative_path: Option<&Path>,
        target_relative_path: Option<&Path>,
    ) -> Result<(), Self::Error> {
        let response = self
            .client
            .post(format!("{}/history/body/restore", self.base_url))
            .json(&RestoreDocumentBodyRevisionRequest {
                workspace_id: binding.workspace_id().as_str().to_owned(),
                actor_id: binding.actor_id().as_str().to_owned(),
                document_id: document_id.as_str().to_owned(),
                seq,
                target_mount_relative_path: target_mount_relative_path
                    .map(|path| path.display().to_string()),
                target_relative_path: target_relative_path.map(|path| path.display().to_string()),
            })
            .send()
            .map_err(io::Error::other)?;

        if !response.status().is_success() {
            return Err(response_error("restore body revision request", response));
        }

        Ok(())
    }

    fn redact_document_body_history(
        &mut self,
        binding: &dyn SyncContext,
        document_id: &DocumentId,
        exact_text: &str,
    ) -> Result<(), Self::Error> {
        let response = self
            .client
            .post(format!("{}/history/body/redact", self.base_url))
            .json(&RedactDocumentBodyHistoryRequest {
                workspace_id: binding.workspace_id().as_str().to_owned(),
                actor_id: binding.actor_id().as_str().to_owned(),
                document_id: document_id.as_str().to_owned(),
                exact_text: exact_text.to_owned(),
            })
            .send()
            .map_err(io::Error::other)?;

        if !response.status().is_success() {
            return Err(response_error("redact body history request", response));
        }

        Ok(())
    }

    fn purge_document_body_history(
        &mut self,
        binding: &dyn SyncContext,
        document_id: &DocumentId,
    ) -> Result<(), Self::Error> {
        let response = self
            .client
            .post(format!("{}/history/body/purge", self.base_url))
            .json(&PurgeDocumentBodyHistoryRequest {
                workspace_id: binding.workspace_id().as_str().to_owned(),
                actor_id: binding.actor_id().as_str().to_owned(),
                document_id: document_id.as_str().to_owned(),
            })
            .send()
            .map_err(io::Error::other)?;

        if !response.status().is_success() {
            return Err(response_error("purge body history request", response));
        }

        Ok(())
    }

    fn restore_workspace_at_cursor(
        &mut self,
        binding: &dyn SyncContext,
        based_on_cursor: u64,
        cursor: u64,
    ) -> Result<(), Self::Error> {
        let response = self
            .client
            .post(format!("{}/history/workspace/restore", self.base_url))
            .json(&RestoreWorkspaceRequest {
                workspace_id: binding.workspace_id().as_str().to_owned(),
                actor_id: binding.actor_id().as_str().to_owned(),
                based_on_cursor: Some(based_on_cursor),
                cursor,
            })
            .send()
            .map_err(io::Error::other)?;

        if !response.status().is_success() {
            return Err(response_error("restore workspace request", response));
        }

        Ok(())
    }

    fn resolve_document_by_historical_path(
        &mut self,
        binding: &dyn SyncContext,
        mount_relative_path: &Path,
        relative_path: &Path,
    ) -> Result<DocumentId, Self::Error> {
        let response = self
            .client
            .post(format!("{}/history/path/resolve", self.base_url))
            .json(&ResolveHistoricalPathRequest {
                workspace_id: binding.workspace_id().as_str().to_owned(),
                mount_relative_path: mount_relative_path.display().to_string(),
                relative_path: relative_path.display().to_string(),
            })
            .send()
            .map_err(io::Error::other)?;

        if !response.status().is_success() {
            return Err(response_error("resolve historical path request", response));
        }

        let payload: ResolveHistoricalPathResponse = response.json().map_err(io::Error::other)?;
        Ok(DocumentId::new(payload.document_id))
    }
}

fn response_error(context: &str, response: reqwest::blocking::Response) -> io::Error {
    let status = response.status();
    let body = response
        .json::<ApiErrorResponse>()
        .ok()
        .map(|payload| format!("{}: {}", payload.code, payload.message))
        .unwrap_or_else(|| "unstructured error".to_owned());
    io::Error::other(format!("{context} failed with status {status}: {body}"))
}
