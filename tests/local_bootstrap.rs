/**
@module PROJECTOR.TESTS.SUPPORT.LOCAL_BOOTSTRAP
Shared local-bootstrap harness, fake transports, and test helpers for projector integration proofs.
*/
// @fileimplements PROJECTOR.TESTS.SUPPORT.LOCAL_BOOTSTRAP
use std::fs;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use projector_domain::{
    ActorId, ApiErrorResponse, BootstrapSnapshot, CheckoutBinding, DocumentBody,
    DocumentBodyHistoryKind, DocumentBodyPurgeMatch, DocumentBodyRedactionMatch,
    DocumentBodyRevision, DocumentId, DocumentKind, DocumentPathEventKind, DocumentPathRevision,
    GetHistoryCompactionPolicyRequest, GetHistoryCompactionPolicyResponse, HistoryCompactionPolicy,
    HistoryCompactionPolicySourceKind, ListBodyRevisionsRequest, ListBodyRevisionsResponse,
    ListEventsRequest, ListEventsResponse, ListPathRevisionsRequest, ListPathRevisionsResponse,
    ManifestEntry, ManifestState, PreviewPurgeDocumentBodyHistoryRequest,
    PreviewPurgeDocumentBodyHistoryResponse, PreviewRedactDocumentBodyHistoryRequest,
    PreviewRedactDocumentBodyHistoryResponse, ProjectionRoots, ProvenanceEvent,
    ProvenanceEventKind, PurgeDocumentBodyHistoryRequest, ReconstructWorkspaceRequest,
    ReconstructWorkspaceResponse, RedactDocumentBodyHistoryRequest, RepoSyncConfig, RepoSyncEntry,
    ResolveHistoricalPathRequest, ResolveHistoricalPathResponse, RestoreWorkspaceRequest,
    SetHistoryCompactionPolicyRequest, SyncContext, SyncEntryKind, WorkspaceId,
};
use projector_runtime::{
    BindingStore, FileBindingStore, FileMachineSyncRegistryStore, FileProvenanceLog,
    FileRepoSyncConfigStore, FileRuntimeStatusStore, FileServerProfileStore, HttpTransport,
    ProjectorHome, RuntimeStatus, StoredEvent, SyncIssueDisposition, SyncLoopOptions, SyncRunner,
    Transport, derive_sync_targets,
};

#[path = "local_bootstrap/binding_support.rs"]
mod binding_support;
#[path = "local_bootstrap/command_harness.rs"]
mod command_harness;
#[path = "local_bootstrap/compact_cli.rs"]
mod compact_cli;
#[path = "local_bootstrap/connection_cli.rs"]
mod connection_cli;
#[path = "local_bootstrap/fake_transport.rs"]
mod fake_transport;
#[path = "local_bootstrap/history_cli.rs"]
mod history_cli;
#[path = "local_bootstrap/history_server.rs"]
mod history_server;
#[path = "local_bootstrap/history_surgery.rs"]
mod history_surgery;
#[path = "local_bootstrap/legacy_sync.rs"]
mod legacy_sync;
#[path = "local_bootstrap/observability_cli.rs"]
mod observability_cli;
#[path = "local_bootstrap/restore_cli.rs"]
mod restore_cli;
#[path = "local_bootstrap/server_api.rs"]
mod server_api;
#[path = "local_bootstrap/sync_bootstrap.rs"]
mod sync_bootstrap;
#[path = "local_bootstrap/sync_entry_cli.rs"]
mod sync_entry_cli;

use binding_support::{
    add_sync_entry, clone_sync_config_for_repo, connect_profile,
    load_workspace_binding_from_sync_config, save_sync_config_for_binding,
};
use command_harness::{
    install_fake_ssh_tools, run_projector, run_projector_failure_with_env, run_projector_home,
    run_projector_tty, run_projector_tty_with_env, run_projector_with_env, spawn_server,
    temp_projector_home, temp_repo,
};
use fake_transport::{
    RejectCreateTransport, RetryAfterRebootstrapTransport, RetryImmediatelyTransport,
};
use legacy_sync::run_legacy_sync_with_env;
use server_api::{
    get_history_compaction_policy_raw, list_body_revisions, list_events, list_path_revisions,
    preview_purge_body_history, preview_redact_body_history, purge_body_history,
    purge_body_history_failure, reconstruct_workspace_at_cursor, redact_body_history,
    redact_body_history_failure, resolve_document_by_historical_path, restore_workspace_at_cursor,
    seed_remote_sync_entry, set_history_compaction_policy_raw,
};
