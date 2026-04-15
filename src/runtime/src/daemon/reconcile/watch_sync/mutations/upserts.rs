/**
@module PROJECTOR.RUNTIME.WATCH_UPSERTS
Applies watcher-driven present-file create and update operations by reading local UTF-8 text bodies, comparing them to the current snapshot, and issuing the appropriate server writes.
*/
// @fileimplements PROJECTOR.RUNTIME.WATCH_UPSERTS
use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::io;

use projector_domain::{DocumentId, ManifestEntry, ProvenanceEventKind, SyncContext};

use super::super::super::super::SyncRunner;
use super::super::moves::TouchedProjectionPath;
use crate::Transport;

pub(super) fn apply_present_file_mutation<C, T>(
    runner: &mut SyncRunner<'_, C, T>,
    transport: &mut T,
    touched_path: &TouchedProjectionPath,
    known_entry: Option<&ManifestEntry>,
    body_by_id: &HashMap<DocumentId, &str>,
    materialized_body_texts: &HashMap<DocumentId, String>,
    manifest_cursor: &mut u64,
    append_event: fn(
        &mut SyncRunner<'_, C, T>,
        ProvenanceEventKind,
        &std::path::Path,
        &std::path::Path,
        &str,
    ) -> Result<(), Box<dyn Error>>,
) -> Result<bool, Box<dyn Error>>
where
    C: SyncContext,
    T: Transport<Error = io::Error>,
{
    if touched_path.absolute_path.exists() && touched_path.absolute_path.is_dir() {
        return Ok(false);
    }

    let local_text = match fs::read_to_string(&touched_path.absolute_path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::InvalidData => return Ok(false),
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err.into()),
    };

    if let Some(entry) = known_entry {
        let remote_text = body_by_id
            .get(&entry.document_id)
            .ok_or_else(|| format!("snapshot missing body for {}", entry.document_id.as_str()))?;
        if local_text == *remote_text {
            return Ok(false);
        }
        let base_text = materialized_body_texts
            .get(&entry.document_id)
            .map(String::as_str)
            .unwrap_or(remote_text);
        transport.update_document(runner.binding, &entry.document_id, base_text, &local_text)?;
        append_event(
            runner,
            ProvenanceEventKind::DocumentUpdated,
            &touched_path.mount_relative_path,
            &touched_path.relative_path,
            "updated local text document through server sync",
        )?;
        return Ok(true);
    }

    transport.create_document(
        runner.binding,
        *manifest_cursor,
        &touched_path.mount_relative_path,
        &touched_path.relative_path,
        &local_text,
    )?;
    *manifest_cursor += 1;
    append_event(
        runner,
        ProvenanceEventKind::DocumentCreated,
        &touched_path.mount_relative_path,
        &touched_path.relative_path,
        "created local text document through server sync",
    )?;
    Ok(true)
}
