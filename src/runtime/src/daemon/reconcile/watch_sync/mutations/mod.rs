/**
@module PROJECTOR.RUNTIME.WATCH_MUTATIONS
Coordinates watcher-driven mutation application by handling move and delete paths locally while delegating present-file create and update behavior to a narrower runtime module.
*/
// @fileimplements PROJECTOR.RUNTIME.WATCH_MUTATIONS
use std::collections::{BTreeSet, HashMap, HashSet};
use std::error::Error;
use std::io;
use std::path::{Path, PathBuf};

use projector_domain::{
    BootstrapSnapshot, DocumentId, ManifestEntry, ProvenanceEventKind, SyncContext,
};

use super::super::super::{SyncRunner, recovery};
use super::super::full_sync::{load_materialized_body_texts, merge_snapshots};
use super::moves::{MoveOperation, TouchedProjectionPath};
use crate::{StoredEvent, Transport};

mod upserts;

pub(super) fn apply_watcher_mutations<C, T>(
    runner: &mut SyncRunner<'_, C, T>,
    transport: &mut T,
    current_snapshot: &BootstrapSnapshot,
    current_cursor: u64,
    touched_paths: BTreeSet<TouchedProjectionPath>,
    move_operations: Vec<MoveOperation>,
) -> Result<Option<(BootstrapSnapshot, u64)>, Box<dyn Error>>
where
    C: SyncContext,
    T: Transport<Error = io::Error>,
{
    let moved_document_ids = move_operations
        .iter()
        .map(|move_op| move_op.document_id.clone())
        .collect::<HashSet<_>>();
    let moved_paths = move_operations
        .iter()
        .map(|move_op| {
            (
                move_op.mount_relative_path.clone(),
                move_op.relative_path.clone(),
            )
        })
        .collect::<BTreeSet<_>>();

    let body_by_id = current_snapshot
        .bodies
        .iter()
        .map(|body| (body.document_id.clone(), body.text.as_str()))
        .collect::<HashMap<_, _>>();
    let materialized_body_texts =
        load_materialized_body_texts(runner.binding.projector_dir()).unwrap_or_default();
    let live_entries = current_snapshot
        .manifest
        .entries
        .iter()
        .filter(|entry| !entry.deleted)
        .map(|entry| {
            (
                (
                    entry.mount_relative_path.clone(),
                    entry.relative_path.clone(),
                ),
                entry,
            )
        })
        .collect::<HashMap<_, _>>();

    let mut changed = false;
    let mut manifest_cursor = current_cursor;

    changed |= apply_move_operations(runner, transport, &move_operations, &mut manifest_cursor)?;
    changed |= apply_non_move_mutations(
        runner,
        transport,
        touched_paths,
        &moved_paths,
        &moved_document_ids,
        &live_entries,
        &body_by_id,
        &materialized_body_texts,
        &mut manifest_cursor,
    )?;

    if !changed {
        return Ok(None);
    }

    let (delta_snapshot, cursor) = transport.changes_since(runner.binding, current_cursor)?;
    Ok(Some((
        merge_snapshots(current_snapshot.clone(), delta_snapshot),
        cursor,
    )))
}

fn apply_move_operations<C, T>(
    runner: &mut SyncRunner<'_, C, T>,
    transport: &mut T,
    move_operations: &[MoveOperation],
    manifest_cursor: &mut u64,
) -> Result<bool, Box<dyn Error>>
where
    C: SyncContext,
    T: Transport<Error = io::Error>,
{
    let mut changed = false;
    for move_op in move_operations {
        transport.move_document(
            runner.binding,
            *manifest_cursor,
            &move_op.document_id,
            &move_op.mount_relative_path,
            &move_op.relative_path,
        )?;
        *manifest_cursor += 1;
        append_event(
            runner,
            ProvenanceEventKind::DocumentMoved,
            &move_op.mount_relative_path,
            &move_op.relative_path,
            "moved local text document through server sync",
        )?;
        changed = true;
    }
    Ok(changed)
}

fn apply_non_move_mutations<C, T>(
    runner: &mut SyncRunner<'_, C, T>,
    transport: &mut T,
    touched_paths: BTreeSet<TouchedProjectionPath>,
    moved_paths: &BTreeSet<(PathBuf, PathBuf)>,
    moved_document_ids: &HashSet<DocumentId>,
    live_entries: &HashMap<(PathBuf, PathBuf), &ManifestEntry>,
    body_by_id: &HashMap<DocumentId, &str>,
    materialized_body_texts: &HashMap<DocumentId, String>,
    manifest_cursor: &mut u64,
) -> Result<bool, Box<dyn Error>>
where
    C: SyncContext,
    T: Transport<Error = io::Error>,
{
    let mut changed = false;

    for touched_path in touched_paths {
        if should_skip_touched_path(&touched_path, moved_paths, moved_document_ids, live_entries) {
            continue;
        }

        let known_entry = live_entries
            .get(&(
                touched_path.mount_relative_path.clone(),
                touched_path.relative_path.clone(),
            ))
            .copied();

        if touched_path.absolute_path.exists() {
            changed |= upserts::apply_present_file_mutation(
                runner,
                transport,
                &touched_path,
                known_entry,
                body_by_id,
                materialized_body_texts,
                manifest_cursor,
                append_event,
            )?;
        } else {
            changed |= apply_deleted_file_mutation(
                runner,
                transport,
                &touched_path,
                known_entry,
                manifest_cursor,
            )?;
        }
    }

    Ok(changed)
}

fn should_skip_touched_path(
    touched_path: &TouchedProjectionPath,
    moved_paths: &BTreeSet<(PathBuf, PathBuf)>,
    moved_document_ids: &HashSet<DocumentId>,
    live_entries: &HashMap<(PathBuf, PathBuf), &ManifestEntry>,
) -> bool {
    if moved_paths.contains(&(
        touched_path.mount_relative_path.clone(),
        touched_path.relative_path.clone(),
    )) {
        return true;
    }

    live_entries
        .get(&(
            touched_path.mount_relative_path.clone(),
            touched_path.relative_path.clone(),
        ))
        .map(|entry| moved_document_ids.contains(&entry.document_id))
        .unwrap_or(false)
}

fn apply_deleted_file_mutation<C, T>(
    runner: &mut SyncRunner<'_, C, T>,
    transport: &mut T,
    touched_path: &TouchedProjectionPath,
    known_entry: Option<&ManifestEntry>,
    manifest_cursor: &mut u64,
) -> Result<bool, Box<dyn Error>>
where
    C: SyncContext,
    T: Transport<Error = io::Error>,
{
    let Some(entry) = known_entry else {
        return Ok(false);
    };

    transport.delete_document(runner.binding, *manifest_cursor, &entry.document_id)?;
    *manifest_cursor += 1;
    append_event(
        runner,
        ProvenanceEventKind::DocumentDeleted,
        &touched_path.mount_relative_path,
        &touched_path.relative_path,
        "deleted local text document through server sync",
    )?;
    Ok(true)
}

pub(super) fn append_event<C, T>(
    runner: &mut SyncRunner<'_, C, T>,
    kind: ProvenanceEventKind,
    mount_relative_path: &Path,
    relative_path: &Path,
    summary: &str,
) -> Result<(), Box<dyn Error>>
where
    C: SyncContext,
    T: Transport<Error = io::Error>,
{
    runner.log.append(&StoredEvent {
        timestamp_ms: recovery::now_ms(),
        actor_id: runner.binding.actor_id().clone(),
        kind,
        path: format!(
            "{}/{}",
            mount_relative_path.display(),
            relative_path.display()
        ),
        summary: summary.to_owned(),
    })?;
    Ok(())
}
