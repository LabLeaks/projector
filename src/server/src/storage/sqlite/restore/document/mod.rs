/**
@module PROJECTOR.SERVER.SQLITE_DOCUMENT_RESTORE
Owns the SQLite document-restore seam over revision selection, path resolution, and restored-history application.
*/
// @fileimplements PROJECTOR.SERVER.SQLITE_DOCUMENT_RESTORE
mod apply;
mod resolve;

use projector_domain::RestoreDocumentBodyRevisionRequest;

use crate::storage::StoreError;

pub(super) fn restore_document_body_revision_tx(
    transaction: &rusqlite::Transaction<'_>,
    request: &RestoreDocumentBodyRevisionRequest,
) -> Result<(), StoreError> {
    let resolution = resolve::resolve_document_restore_target(transaction, request)?;
    apply::apply_document_restore(transaction, request, resolution)
}
