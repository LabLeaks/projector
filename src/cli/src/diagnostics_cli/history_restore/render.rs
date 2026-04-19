/**
@module PROJECTOR.EDGE.HISTORY_RESTORE_RENDERING
Owns terminal rendering helpers shared by history, restore, and log surfaces.
*/
// @fileimplements PROJECTOR.EDGE.HISTORY_RESTORE_RENDERING
use std::path::{Path, PathBuf};

use projector_domain::{BootstrapSnapshot, DocumentBodyRevision, DocumentId, DocumentPathRevision};

use crate::restore_browser::{simple_line_diff, simple_line_diff_with_labels};

pub(crate) fn format_event_path(
    mount_relative_path: Option<&str>,
    relative_path: Option<&str>,
) -> String {
    match (mount_relative_path, relative_path) {
        (Some(mount), Some(relative)) if relative.is_empty() => mount.to_owned(),
        (Some(mount), Some(relative)) => format!("{mount}/{relative}"),
        (Some(mount), None) => mount.to_owned(),
        (None, Some(relative)) => relative.to_owned(),
        (None, None) => "-".to_owned(),
    }
}

pub(super) fn print_body_revision(revision: &DocumentBodyRevision) {
    println!(
        "body_revision: seq={} actor={} kind={} checkpoint_anchor_seq={} conflicted={} timestamp_ms={}",
        revision.seq,
        revision.actor_id,
        revision.history_kind,
        revision
            .checkpoint_anchor_seq
            .map(|seq| seq.to_string())
            .unwrap_or_else(|| "-".to_owned()),
        revision.conflicted,
        revision.timestamp_ms
    );
    println!("snapshot_text: {:?}", revision.body_text);
    println!("diff:");
    for line in
        simple_line_diff_with_labels("base", "snapshot", &revision.base_text, &revision.body_text)
    {
        println!("{line}");
    }
}

pub(super) fn print_path_revision(revision: &DocumentPathRevision) {
    println!(
        "path_revision: seq={} actor={} kind={} deleted={} path={} timestamp_ms={}",
        revision.seq,
        revision.actor_id,
        revision.event_kind,
        revision.deleted,
        Path::new(&revision.mount_path)
            .join(&revision.relative_path)
            .display(),
        revision.timestamp_ms
    );
}

pub(super) fn print_restore_preview(
    path: &Path,
    document_id: &DocumentId,
    restore_seq: u64,
    current_text: &str,
    restored_text: &str,
) {
    println!("path: {}", path.display());
    println!("document_id: {}", document_id.as_str());
    println!("restore_seq: {}", restore_seq);
    println!("preview:");
    let diff = simple_line_diff(current_text, restored_text);
    if diff.is_empty() {
        println!("  (no content change)");
    } else {
        for line in diff {
            println!("{line}");
        }
    }
}

pub(super) fn print_workspace_snapshot(cursor: u64, snapshot: &BootstrapSnapshot) {
    println!("workspace_cursor: {}", cursor);
    println!("manifest_entries: {}", snapshot.manifest.entries.len());
    for entry in &snapshot.manifest.entries {
        println!(
            "manifest_entry: document_id={} deleted={} path={}",
            entry.document_id.as_str(),
            entry.deleted,
            entry
                .mount_relative_path
                .join(&entry.relative_path)
                .display()
        );
    }
    println!("body_documents: {}", snapshot.bodies.len());
    for body in &snapshot.bodies {
        let path = snapshot
            .manifest
            .entries
            .iter()
            .find(|entry| entry.document_id == body.document_id)
            .map(|entry| entry.mount_relative_path.join(&entry.relative_path))
            .unwrap_or_else(|| PathBuf::from(body.document_id.as_str()));
        println!(
            "body_document: document_id={} path={} text={:?}",
            body.document_id.as_str(),
            path.display(),
            body.text
        );
    }
}

#[cfg(test)]
mod tests {
    use super::format_event_path;

    #[test]
    fn format_event_path_omits_trailing_separator_for_file_root() {
        assert_eq!(format_event_path(Some("AGENTS.md"), Some("")), "AGENTS.md");
    }
}
