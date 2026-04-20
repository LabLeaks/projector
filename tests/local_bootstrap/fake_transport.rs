/**
@module PROJECTOR.TESTS.SUPPORT.LOCAL_BOOTSTRAP_FAKE_TRANSPORT
Fake transport implementations for local-bootstrap sync recovery and retry proofs.
*/
// @fileimplements PROJECTOR.TESTS.SUPPORT.LOCAL_BOOTSTRAP_FAKE_TRANSPORT
use super::*;

pub(crate) fn default_compaction_policy_response() -> (
    HistoryCompactionPolicy,
    HistoryCompactionPolicySourceKind,
    Option<String>,
) {
    (
        HistoryCompactionPolicy {
            revisions: 100,
            frequency: 10,
        },
        HistoryCompactionPolicySourceKind::Default,
        None,
    )
}

#[derive(Clone, Debug)]
pub(crate) struct RejectCreateTransport;

impl Transport for RejectCreateTransport {
    type Error = io::Error;

    fn bootstrap(
        &mut self,
        _binding: &dyn SyncContext,
    ) -> Result<(BootstrapSnapshot, u64), Self::Error> {
        Ok((BootstrapSnapshot::default(), 0))
    }

    fn changes_since(
        &mut self,
        _binding: &dyn SyncContext,
        _since_cursor: u64,
    ) -> Result<(BootstrapSnapshot, u64), Self::Error> {
        Ok((BootstrapSnapshot::default(), 0))
    }

    fn create_document(
        &mut self,
        _binding: &dyn SyncContext,
        _based_on_cursor: u64,
        _mount_relative_path: &Path,
        _relative_path: &Path,
        _text: &str,
    ) -> Result<DocumentId, Self::Error> {
        Err(io::Error::other(
            "create document request failed with status 409 Conflict: stale_cursor: manifest write based on stale cursor 0; current workspace cursor is 1",
        ))
    }

    fn update_document(
        &mut self,
        _binding: &dyn SyncContext,
        _document_id: &DocumentId,
        _base_text: &str,
        _text: &str,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn delete_document(
        &mut self,
        _binding: &dyn SyncContext,
        _based_on_cursor: u64,
        _document_id: &DocumentId,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn move_document(
        &mut self,
        _binding: &dyn SyncContext,
        _based_on_cursor: u64,
        _document_id: &DocumentId,
        _mount_relative_path: &Path,
        _relative_path: &Path,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn provenance(
        &mut self,
        _binding: &dyn SyncContext,
        _limit: usize,
    ) -> Result<Vec<ProvenanceEvent>, Self::Error> {
        Ok(Vec::new())
    }

    fn list_body_revisions(
        &mut self,
        _binding: &dyn SyncContext,
        _document_id: &DocumentId,
        _limit: usize,
    ) -> Result<Vec<DocumentBodyRevision>, Self::Error> {
        Ok(Vec::new())
    }

    fn preview_redact_document_body_history(
        &mut self,
        _binding: &dyn SyncContext,
        _document_id: &DocumentId,
        _exact_text: &str,
        _limit: usize,
    ) -> Result<Vec<DocumentBodyRedactionMatch>, Self::Error> {
        Ok(Vec::new())
    }

    fn preview_purge_document_body_history(
        &mut self,
        _binding: &dyn SyncContext,
        _document_id: &DocumentId,
        _limit: usize,
    ) -> Result<Vec<DocumentBodyPurgeMatch>, Self::Error> {
        Ok(Vec::new())
    }

    fn list_path_revisions(
        &mut self,
        _binding: &dyn SyncContext,
        _document_id: &DocumentId,
        _limit: usize,
    ) -> Result<Vec<DocumentPathRevision>, Self::Error> {
        Ok(Vec::new())
    }

    fn reconstruct_workspace_at_cursor(
        &mut self,
        _binding: &dyn SyncContext,
        _cursor: u64,
    ) -> Result<BootstrapSnapshot, Self::Error> {
        Ok(BootstrapSnapshot::default())
    }

    fn restore_workspace_at_cursor(
        &mut self,
        _binding: &dyn SyncContext,
        _based_on_cursor: u64,
        _cursor: u64,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn restore_document_body_revision(
        &mut self,
        _binding: &dyn SyncContext,
        _document_id: &DocumentId,
        _seq: u64,
        _target_mount_relative_path: Option<&Path>,
        _target_relative_path: Option<&Path>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn redact_document_body_history(
        &mut self,
        _binding: &dyn SyncContext,
        _document_id: &DocumentId,
        _exact_text: &str,
        _expected_match_seqs: Option<&[u64]>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn purge_document_body_history(
        &mut self,
        _binding: &dyn SyncContext,
        _document_id: &DocumentId,
        _expected_match_seqs: Option<&[u64]>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn resolve_document_by_historical_path(
        &mut self,
        _binding: &dyn SyncContext,
        _mount_relative_path: &Path,
        _relative_path: &Path,
    ) -> Result<DocumentId, Self::Error> {
        Ok(DocumentId::new("doc-historical"))
    }

    fn get_history_compaction_policy(
        &mut self,
        _binding: &dyn SyncContext,
        _repo_relative_path: &Path,
    ) -> Result<
        (
            HistoryCompactionPolicy,
            HistoryCompactionPolicySourceKind,
            Option<String>,
        ),
        Self::Error,
    > {
        Ok(default_compaction_policy_response())
    }

    fn set_history_compaction_policy(
        &mut self,
        _binding: &dyn SyncContext,
        _repo_relative_path: &Path,
        _policy: &HistoryCompactionPolicy,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn clear_history_compaction_policy(
        &mut self,
        _binding: &dyn SyncContext,
        _repo_relative_path: &Path,
    ) -> Result<bool, Self::Error> {
        Ok(false)
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct RetryAfterRebootstrapTransport {
    create_attempts: usize,
    created: Option<(DocumentId, String)>,
}

impl Transport for RetryAfterRebootstrapTransport {
    type Error = io::Error;

    fn bootstrap(
        &mut self,
        _binding: &dyn SyncContext,
    ) -> Result<(BootstrapSnapshot, u64), Self::Error> {
        let mut snapshot = BootstrapSnapshot::default();
        let mut cursor = 0;
        if let Some((document_id, text)) = &self.created {
            snapshot.manifest.entries.push(ManifestEntry {
                document_id: document_id.clone(),
                mount_relative_path: PathBuf::from("private"),
                relative_path: PathBuf::from("briefs/retry.html"),
                kind: DocumentKind::Text,
                deleted: false,
            });
            snapshot.bodies.push(DocumentBody {
                document_id: document_id.clone(),
                text: text.clone(),
            });
            cursor = 1;
        }
        Ok((snapshot, cursor))
    }

    fn changes_since(
        &mut self,
        _binding: &dyn SyncContext,
        since_cursor: u64,
    ) -> Result<(BootstrapSnapshot, u64), Self::Error> {
        if since_cursor >= 1 {
            return Ok((BootstrapSnapshot::default(), since_cursor));
        }
        self.bootstrap(_binding)
    }

    fn create_document(
        &mut self,
        _binding: &dyn SyncContext,
        _based_on_cursor: u64,
        _mount_relative_path: &Path,
        _relative_path: &Path,
        text: &str,
    ) -> Result<DocumentId, Self::Error> {
        self.create_attempts += 1;
        if self.create_attempts == 1 {
            return Err(io::Error::other(
                "create document request failed with status 409 Conflict: stale_cursor: manifest write based on stale cursor 0; current workspace cursor is 1",
            ));
        }

        let document_id = DocumentId::new("doc-retried");
        self.created = Some((document_id.clone(), text.to_owned()));
        Ok(document_id)
    }

    fn update_document(
        &mut self,
        _binding: &dyn SyncContext,
        _document_id: &DocumentId,
        _base_text: &str,
        _text: &str,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn delete_document(
        &mut self,
        _binding: &dyn SyncContext,
        _based_on_cursor: u64,
        _document_id: &DocumentId,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn move_document(
        &mut self,
        _binding: &dyn SyncContext,
        _based_on_cursor: u64,
        _document_id: &DocumentId,
        _mount_relative_path: &Path,
        _relative_path: &Path,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn provenance(
        &mut self,
        _binding: &dyn SyncContext,
        _limit: usize,
    ) -> Result<Vec<ProvenanceEvent>, Self::Error> {
        Ok(Vec::new())
    }

    fn list_body_revisions(
        &mut self,
        _binding: &dyn SyncContext,
        _document_id: &DocumentId,
        _limit: usize,
    ) -> Result<Vec<DocumentBodyRevision>, Self::Error> {
        Ok(Vec::new())
    }

    fn preview_redact_document_body_history(
        &mut self,
        _binding: &dyn SyncContext,
        _document_id: &DocumentId,
        _exact_text: &str,
        _limit: usize,
    ) -> Result<Vec<DocumentBodyRedactionMatch>, Self::Error> {
        Ok(Vec::new())
    }

    fn preview_purge_document_body_history(
        &mut self,
        _binding: &dyn SyncContext,
        _document_id: &DocumentId,
        _limit: usize,
    ) -> Result<Vec<DocumentBodyPurgeMatch>, Self::Error> {
        Ok(Vec::new())
    }

    fn list_path_revisions(
        &mut self,
        _binding: &dyn SyncContext,
        _document_id: &DocumentId,
        _limit: usize,
    ) -> Result<Vec<DocumentPathRevision>, Self::Error> {
        Ok(Vec::new())
    }

    fn reconstruct_workspace_at_cursor(
        &mut self,
        _binding: &dyn SyncContext,
        _cursor: u64,
    ) -> Result<BootstrapSnapshot, Self::Error> {
        self.bootstrap(_binding).map(|(snapshot, _)| snapshot)
    }

    fn restore_workspace_at_cursor(
        &mut self,
        _binding: &dyn SyncContext,
        _based_on_cursor: u64,
        _cursor: u64,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn restore_document_body_revision(
        &mut self,
        _binding: &dyn SyncContext,
        _document_id: &DocumentId,
        _seq: u64,
        _target_mount_relative_path: Option<&Path>,
        _target_relative_path: Option<&Path>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn redact_document_body_history(
        &mut self,
        _binding: &dyn SyncContext,
        _document_id: &DocumentId,
        _exact_text: &str,
        _expected_match_seqs: Option<&[u64]>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn purge_document_body_history(
        &mut self,
        _binding: &dyn SyncContext,
        _document_id: &DocumentId,
        _expected_match_seqs: Option<&[u64]>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn resolve_document_by_historical_path(
        &mut self,
        _binding: &dyn SyncContext,
        _mount_relative_path: &Path,
        _relative_path: &Path,
    ) -> Result<DocumentId, Self::Error> {
        Ok(DocumentId::new("doc-historical"))
    }

    fn get_history_compaction_policy(
        &mut self,
        _binding: &dyn SyncContext,
        _repo_relative_path: &Path,
    ) -> Result<
        (
            HistoryCompactionPolicy,
            HistoryCompactionPolicySourceKind,
            Option<String>,
        ),
        Self::Error,
    > {
        Ok(default_compaction_policy_response())
    }

    fn set_history_compaction_policy(
        &mut self,
        _binding: &dyn SyncContext,
        _repo_relative_path: &Path,
        _policy: &HistoryCompactionPolicy,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn clear_history_compaction_policy(
        &mut self,
        _binding: &dyn SyncContext,
        _repo_relative_path: &Path,
    ) -> Result<bool, Self::Error> {
        Ok(false)
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct RetryImmediatelyTransport {
    create_attempts: usize,
    created: Option<(DocumentId, String)>,
}

impl Transport for RetryImmediatelyTransport {
    type Error = io::Error;

    fn bootstrap(
        &mut self,
        _binding: &dyn SyncContext,
    ) -> Result<(BootstrapSnapshot, u64), Self::Error> {
        let mut snapshot = BootstrapSnapshot::default();
        let mut cursor = 0;
        if let Some((document_id, text)) = &self.created {
            snapshot.manifest.entries.push(ManifestEntry {
                document_id: document_id.clone(),
                mount_relative_path: PathBuf::from("private"),
                relative_path: PathBuf::from("briefs/transient.html"),
                kind: DocumentKind::Text,
                deleted: false,
            });
            snapshot.bodies.push(DocumentBody {
                document_id: document_id.clone(),
                text: text.clone(),
            });
            cursor = 1;
        }
        Ok((snapshot, cursor))
    }

    fn changes_since(
        &mut self,
        binding: &dyn SyncContext,
        since_cursor: u64,
    ) -> Result<(BootstrapSnapshot, u64), Self::Error> {
        if since_cursor >= 1 {
            return Ok((BootstrapSnapshot::default(), since_cursor));
        }
        self.bootstrap(binding)
    }

    fn create_document(
        &mut self,
        _binding: &dyn SyncContext,
        _based_on_cursor: u64,
        _mount_relative_path: &Path,
        _relative_path: &Path,
        text: &str,
    ) -> Result<DocumentId, Self::Error> {
        self.create_attempts += 1;
        if self.create_attempts == 1 {
            return Err(io::Error::other("tcp connect error: connection refused"));
        }

        let document_id = DocumentId::new("doc-transient");
        self.created = Some((document_id.clone(), text.to_owned()));
        Ok(document_id)
    }

    fn update_document(
        &mut self,
        _binding: &dyn SyncContext,
        _document_id: &DocumentId,
        _base_text: &str,
        _text: &str,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn delete_document(
        &mut self,
        _binding: &dyn SyncContext,
        _based_on_cursor: u64,
        _document_id: &DocumentId,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn move_document(
        &mut self,
        _binding: &dyn SyncContext,
        _based_on_cursor: u64,
        _document_id: &DocumentId,
        _mount_relative_path: &Path,
        _relative_path: &Path,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn provenance(
        &mut self,
        _binding: &dyn SyncContext,
        _limit: usize,
    ) -> Result<Vec<ProvenanceEvent>, Self::Error> {
        Ok(Vec::new())
    }

    fn list_body_revisions(
        &mut self,
        _binding: &dyn SyncContext,
        _document_id: &DocumentId,
        _limit: usize,
    ) -> Result<Vec<DocumentBodyRevision>, Self::Error> {
        Ok(Vec::new())
    }

    fn preview_redact_document_body_history(
        &mut self,
        _binding: &dyn SyncContext,
        _document_id: &DocumentId,
        _exact_text: &str,
        _limit: usize,
    ) -> Result<Vec<DocumentBodyRedactionMatch>, Self::Error> {
        Ok(Vec::new())
    }

    fn preview_purge_document_body_history(
        &mut self,
        _binding: &dyn SyncContext,
        _document_id: &DocumentId,
        _limit: usize,
    ) -> Result<Vec<DocumentBodyPurgeMatch>, Self::Error> {
        Ok(Vec::new())
    }

    fn list_path_revisions(
        &mut self,
        _binding: &dyn SyncContext,
        _document_id: &DocumentId,
        _limit: usize,
    ) -> Result<Vec<DocumentPathRevision>, Self::Error> {
        Ok(Vec::new())
    }

    fn reconstruct_workspace_at_cursor(
        &mut self,
        _binding: &dyn SyncContext,
        _cursor: u64,
    ) -> Result<BootstrapSnapshot, Self::Error> {
        self.bootstrap(_binding).map(|(snapshot, _)| snapshot)
    }

    fn restore_workspace_at_cursor(
        &mut self,
        _binding: &dyn SyncContext,
        _based_on_cursor: u64,
        _cursor: u64,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn restore_document_body_revision(
        &mut self,
        _binding: &dyn SyncContext,
        _document_id: &DocumentId,
        _seq: u64,
        _target_mount_relative_path: Option<&Path>,
        _target_relative_path: Option<&Path>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn redact_document_body_history(
        &mut self,
        _binding: &dyn SyncContext,
        _document_id: &DocumentId,
        _exact_text: &str,
        _expected_match_seqs: Option<&[u64]>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn purge_document_body_history(
        &mut self,
        _binding: &dyn SyncContext,
        _document_id: &DocumentId,
        _expected_match_seqs: Option<&[u64]>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn resolve_document_by_historical_path(
        &mut self,
        _binding: &dyn SyncContext,
        _mount_relative_path: &Path,
        _relative_path: &Path,
    ) -> Result<DocumentId, Self::Error> {
        Ok(DocumentId::new("doc-historical"))
    }

    fn get_history_compaction_policy(
        &mut self,
        _binding: &dyn SyncContext,
        _repo_relative_path: &Path,
    ) -> Result<
        (
            HistoryCompactionPolicy,
            HistoryCompactionPolicySourceKind,
            Option<String>,
        ),
        Self::Error,
    > {
        Ok(default_compaction_policy_response())
    }

    fn set_history_compaction_policy(
        &mut self,
        _binding: &dyn SyncContext,
        _repo_relative_path: &Path,
        _policy: &HistoryCompactionPolicy,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn clear_history_compaction_policy(
        &mut self,
        _binding: &dyn SyncContext,
        _repo_relative_path: &Path,
    ) -> Result<bool, Self::Error> {
        Ok(false)
    }
}
