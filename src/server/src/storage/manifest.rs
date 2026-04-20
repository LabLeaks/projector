/**
@module PROJECTOR.SERVER.MANIFEST
Owns shared document path lifecycle helpers and stale-cursor checks while delegating file-backed and Postgres-backed manifest mutations to narrower backend modules.
*/
// @fileimplements PROJECTOR.SERVER.MANIFEST
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use super::StoreError;
use super::provenance::current_workspace_cursor_tx;

pub(crate) use super::manifest_file::*;
pub(crate) use super::manifest_postgres::*;

static MANIFEST_FALLBACK_COUNTER: AtomicU64 = AtomicU64::new(1);

pub(crate) fn display_document_path(mount_path: &str, relative_path: &str) -> String {
    if relative_path.is_empty() {
        mount_path.to_owned()
    } else {
        format!("{mount_path}/{relative_path}")
    }
}

pub(crate) fn file_enforce_manifest_cursor(
    state_dir: &Path,
    workspace_id: &str,
    based_on_cursor: Option<u64>,
) -> Result<(), StoreError> {
    let provided = based_on_cursor
        .ok_or_else(|| StoreError::new("manifest write missing based_on_cursor precondition"))?;
    let current = super::provenance::file_workspace_cursor(state_dir, workspace_id)?;
    if provided == current {
        return Ok(());
    }

    Err(StoreError::conflict(
        "stale_cursor",
        format!(
            "manifest write based on stale cursor {provided}; current workspace cursor is {current}"
        ),
    ))
}

pub(crate) async fn enforce_manifest_cursor_tx(
    transaction: &tokio_postgres::Transaction<'_>,
    workspace_id: &str,
    based_on_cursor: Option<u64>,
) -> Result<(), StoreError> {
    let provided = based_on_cursor
        .ok_or_else(|| StoreError::new("manifest write missing based_on_cursor precondition"))?;
    let current = current_workspace_cursor_tx(transaction, workspace_id).await?;
    if provided == current {
        return Ok(());
    }

    Err(StoreError::conflict(
        "stale_cursor",
        format!(
            "manifest write based on stale cursor {provided}; current workspace cursor is {current}"
        ),
    ))
}

pub(crate) fn make_document_id() -> String {
    let counter = MANIFEST_FALLBACK_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(counter as u128);
    format!("doc-{nanos}-{counter}")
}

pub(crate) fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_else(|_| MANIFEST_FALLBACK_COUNTER.fetch_add(1, Ordering::Relaxed) as u128)
}
