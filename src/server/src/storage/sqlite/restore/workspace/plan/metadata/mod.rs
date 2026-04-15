/**
@module PROJECTOR.SERVER.SQLITE_WORKSPACE_RESTORE_METADATA
Coordinates SQLite workspace-restore event classification and summary rendering for each per-document restore change.
*/
// @fileimplements PROJECTOR.SERVER.SQLITE_WORKSPACE_RESTORE_METADATA
use projector_domain::{ManifestEntry, ProvenanceEventKind};

mod classify;
mod summary;

use classify::classify_restore_change;
use summary::restore_change_summary;

pub(super) fn restore_change_metadata(
    restored_entry: &ManifestEntry,
    current_live: bool,
    restored_live: bool,
    path_changed: bool,
    body_changed: bool,
    target_cursor: u64,
) -> Option<(ProvenanceEventKind, String, String)> {
    let change = classify_restore_change(current_live, restored_live, path_changed, body_changed)?;
    let summary = restore_change_summary(&change, restored_entry, target_cursor);
    Some((
        change.provenance_kind(),
        summary,
        change.summary_code().to_owned(),
    ))
}
