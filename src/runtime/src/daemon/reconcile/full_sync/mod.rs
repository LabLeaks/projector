/**
@module PROJECTOR.RUNTIME.FULL_SYNC
Coordinates snapshot bootstrap reconciliation by delegating full-sync move detection, local text lifecycle pushes, and materialized-path checkpoint persistence to narrower runtime modules.
*/
// @fileimplements PROJECTOR.RUNTIME.FULL_SYNC
use std::collections::HashMap;
use std::error::Error;
use std::io;

use projector_domain::{BootstrapSnapshot, ProvenanceEventKind, SyncContext};

use super::super::{SyncRunner, recovery};
use crate::{StoredEvent, Transport};

mod discovery;
mod local_text;
mod materialized_bodies;
mod materialized_paths;
mod moves;

pub(super) use materialized_bodies::{load_materialized_body_texts, save_materialized_body_texts};
pub(super) use materialized_paths::{load_materialized_paths, save_materialized_paths};

pub(super) fn reconcile_snapshot<C, T>(
    runner: &mut SyncRunner<'_, C, T>,
    transport: &mut T,
) -> Result<(BootstrapSnapshot, u64), Box<dyn Error>>
where
    C: SyncContext,
    T: Transport<Error = io::Error>,
{
    let previously_materialized_paths =
        materialized_paths::load_materialized_paths(runner.binding.projector_dir())
            .unwrap_or_default();
    let (mut snapshot, mut cursor) = transport.bootstrap(runner.binding)?;

    let local_moves =
        moves::detect_full_sync_moves(runner.binding, &snapshot, &previously_materialized_paths)?;
    if !local_moves.is_empty() {
        let mut manifest_cursor = cursor;
        for move_op in &local_moves {
            transport.move_document(
                runner.binding,
                manifest_cursor,
                &move_op.document_id,
                &move_op.mount_relative_path,
                &move_op.relative_path,
            )?;
            manifest_cursor += 1;
            append_event(
                runner,
                ProvenanceEventKind::DocumentMoved,
                &move_op.mount_relative_path,
                &move_op.relative_path,
                "moved local text document through server sync",
            )?;
        }
        refresh_snapshot_from_delta(runner, transport, &mut snapshot, &mut cursor)?;
    }

    let (local_creations, _) =
        local_text::push_local_only_text_documents(runner.binding, &snapshot, cursor, transport)?;
    if !local_creations.is_empty() {
        append_events(
            runner,
            ProvenanceEventKind::DocumentCreated,
            &local_creations,
            "created local text document through server sync",
        )?;
        refresh_snapshot_from_delta(runner, transport, &mut snapshot, &mut cursor)?;
    }

    let (local_deletions, _) = local_text::push_local_text_deletions(
        runner.binding,
        &snapshot,
        &previously_materialized_paths,
        cursor,
        transport,
    )?;
    if !local_deletions.is_empty() {
        append_events(
            runner,
            ProvenanceEventKind::DocumentDeleted,
            &local_deletions,
            "deleted local text document through server sync",
        )?;
        refresh_snapshot_from_delta(runner, transport, &mut snapshot, &mut cursor)?;
    }

    let local_updates = local_text::push_local_text_updates(runner.binding, &snapshot, transport)?;
    if !local_updates.is_empty() {
        append_events(
            runner,
            ProvenanceEventKind::DocumentUpdated,
            &local_updates,
            "updated local text document through server sync",
        )?;
        refresh_snapshot_from_delta(runner, transport, &mut snapshot, &mut cursor)?;
    }

    Ok((snapshot, cursor))
}

fn refresh_snapshot_from_delta<C, T>(
    runner: &SyncRunner<'_, C, T>,
    transport: &mut T,
    snapshot: &mut BootstrapSnapshot,
    cursor: &mut u64,
) -> Result<(), Box<dyn Error>>
where
    C: SyncContext,
    T: Transport<Error = io::Error>,
{
    let delta = transport.changes_since(runner.binding, *cursor)?;
    *snapshot = merge_snapshots(std::mem::take(snapshot), delta.0);
    *cursor = delta.1;
    Ok(())
}

fn append_events<C, T>(
    runner: &mut SyncRunner<'_, C, T>,
    kind: ProvenanceEventKind,
    paths: &[(std::path::PathBuf, std::path::PathBuf)],
    summary: &str,
) -> Result<(), Box<dyn Error>>
where
    C: SyncContext,
    T: Transport<Error = io::Error>,
{
    for (mount_relative_path, relative_path) in paths {
        append_event(
            runner,
            kind.clone(),
            mount_relative_path,
            relative_path,
            summary,
        )?;
    }
    Ok(())
}

fn append_event<C, T>(
    runner: &mut SyncRunner<'_, C, T>,
    kind: ProvenanceEventKind,
    mount_relative_path: &std::path::Path,
    relative_path: &std::path::Path,
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

pub(super) fn merge_snapshots(
    mut current: BootstrapSnapshot,
    delta: BootstrapSnapshot,
) -> BootstrapSnapshot {
    let mut entries_by_id = current
        .manifest
        .entries
        .into_iter()
        .map(|entry| (entry.document_id.clone(), entry))
        .collect::<HashMap<_, _>>();
    for entry in delta.manifest.entries {
        entries_by_id.insert(entry.document_id.clone(), entry);
    }

    let mut bodies_by_id = current
        .bodies
        .into_iter()
        .map(|body| (body.document_id.clone(), body))
        .collect::<HashMap<_, _>>();
    for body in delta.bodies {
        bodies_by_id.insert(body.document_id.clone(), body);
    }

    for entry in entries_by_id.values() {
        if entry.deleted {
            bodies_by_id.remove(&entry.document_id);
        }
    }

    current.manifest.entries = entries_by_id.into_values().collect();
    current
        .manifest
        .entries
        .sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    current.bodies = bodies_by_id.into_values().collect();
    current
        .bodies
        .sort_by(|left, right| left.document_id.as_str().cmp(right.document_id.as_str()));
    current
}
