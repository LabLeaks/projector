/**
@module PROJECTOR.SERVER.SQLITE_WORKSPACE_RESTORE
Coordinates SQLite workspace reconstruction and rewind by delegating snapshot reconstruction, restore planning, and restore application to narrower modules.
*/
// @fileimplements PROJECTOR.SERVER.SQLITE_WORKSPACE_RESTORE
mod apply;
mod merge;
mod plan;
mod reconstruct;

use projector_domain::{BootstrapSnapshot, RestoreWorkspaceRequest};

use crate::storage::StoreError;

pub(super) fn reconstruct_workspace_at_cursor(
    connection: &rusqlite::Connection,
    workspace_id: &str,
    cursor: u64,
) -> Result<BootstrapSnapshot, StoreError> {
    reconstruct::reconstruct_workspace_at_cursor(connection, workspace_id, cursor)
}

pub(super) fn restore_workspace_at_cursor_tx(
    transaction: &rusqlite::Transaction<'_>,
    request: &RestoreWorkspaceRequest,
) -> Result<(), StoreError> {
    apply::restore_workspace_at_cursor_tx(transaction, request)
}
