/**
@module PROJECTOR.SERVER.SQLITE_RESTORE
Coordinates SQLite document restore and workspace rewind operations over persisted path and body history.
*/
// @fileimplements PROJECTOR.SERVER.SQLITE_RESTORE
mod document;
mod workspace;

use projector_domain::{
    BootstrapSnapshot, RestoreDocumentBodyRevisionRequest, RestoreWorkspaceRequest,
};

use super::super::StoreError;

pub(super) fn reconstruct_workspace_at_cursor(
    connection: &rusqlite::Connection,
    workspace_id: &str,
    cursor: u64,
) -> Result<BootstrapSnapshot, StoreError> {
    workspace::reconstruct_workspace_at_cursor(connection, workspace_id, cursor)
}

pub(super) fn restore_workspace_at_cursor_tx(
    transaction: &rusqlite::Transaction<'_>,
    request: &RestoreWorkspaceRequest,
) -> Result<(), StoreError> {
    workspace::restore_workspace_at_cursor_tx(transaction, request)
}

pub(super) fn restore_document_body_revision_tx(
    transaction: &rusqlite::Transaction<'_>,
    request: &RestoreDocumentBodyRevisionRequest,
) -> Result<(), StoreError> {
    document::restore_document_body_revision_tx(transaction, request)
}
