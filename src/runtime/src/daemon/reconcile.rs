/**
@module PROJECTOR.RUNTIME.RECONCILER
Coordinates runtime reconciliation by delegating full-sync bootstrap reconciliation and watch-path mutation reconciliation to narrower runtime modules.
*/
// @fileimplements PROJECTOR.RUNTIME.RECONCILER
use std::error::Error;
use std::io;

use projector_domain::{BootstrapSnapshot, SyncContext};

use super::SyncRunner;
use crate::{Materializer, Transport, WatcherEvent};

mod full_sync;
mod watch_sync;

impl<C, T> SyncRunner<'_, C, T>
where
    C: SyncContext,
    T: Transport<Error = io::Error>,
{
    pub(super) fn reconcile_snapshot(
        &mut self,
    ) -> Result<(BootstrapSnapshot, u64), Box<dyn Error>> {
        let Some(mut transport) = self.transport.take() else {
            return Ok((BootstrapSnapshot::default(), 0));
        };
        let result = full_sync::reconcile_snapshot(self, &mut transport);
        self.transport = Some(transport);
        result
    }

    pub(super) fn apply_snapshot(
        &self,
        snapshot: &BootstrapSnapshot,
    ) -> Result<(), Box<dyn Error>> {
        let known_mounts = self
            .binding
            .projection_mounts()
            .into_iter()
            .map(|mount| mount.relative_path)
            .collect::<std::collections::BTreeSet<_>>();
        self.materializer.ensure_projection_roots()?;
        let mut plan = self.materializer.plan(snapshot)?;
        let previous_paths =
            full_sync::load_materialized_paths(self.binding.projector_dir()).unwrap_or_default();
        let current_live_paths = snapshot
            .manifest
            .entries
            .iter()
            .filter(|entry| !entry.deleted)
            .map(|entry| {
                (
                    entry.mount_relative_path.clone(),
                    entry.relative_path.clone(),
                )
            })
            .collect::<std::collections::BTreeSet<_>>();
        for (mount_relative_path, relative_path) in previous_paths {
            if !known_mounts.contains(&mount_relative_path) {
                continue;
            }
            if current_live_paths.contains(&(mount_relative_path.clone(), relative_path.clone())) {
                continue;
            }
            plan.files_to_remove.push(
                self.materializer
                    .resolve_projection_path(&mount_relative_path, &relative_path)?,
            );
        }
        plan.files_to_remove.sort();
        plan.files_to_remove.dedup();
        self.materializer.apply(&plan)?;
        full_sync::save_materialized_paths(self.binding.projector_dir(), snapshot)?;
        full_sync::save_materialized_body_texts(self.binding.projector_dir(), snapshot)?;
        Ok(())
    }

    pub(super) fn pull_remote_snapshot_if_changed(
        &mut self,
        current_cursor: u64,
        current_snapshot: &BootstrapSnapshot,
    ) -> Result<Option<(BootstrapSnapshot, u64)>, Box<dyn Error>> {
        let Some(transport) = self.transport.as_mut() else {
            return Ok(None);
        };

        let (delta_snapshot, cursor) = transport.changes_since(self.binding, current_cursor)?;
        if cursor == current_cursor {
            return Ok(None);
        }

        Ok(Some((
            full_sync::merge_snapshots(current_snapshot.clone(), delta_snapshot),
            cursor,
        )))
    }

    pub(super) fn push_watcher_events(
        &mut self,
        current_snapshot: &BootstrapSnapshot,
        current_cursor: u64,
        events: &[WatcherEvent],
    ) -> Result<Option<(BootstrapSnapshot, u64)>, Box<dyn Error>> {
        let Some(mut transport) = self.transport.take() else {
            return Ok(None);
        };
        let result = watch_sync::push_watcher_events(
            self,
            &mut transport,
            current_snapshot,
            current_cursor,
            events,
        );
        self.transport = Some(transport);
        result
    }
}

pub(super) fn save_snapshot_checkpoints(
    projector_dir: &std::path::Path,
    snapshot: &BootstrapSnapshot,
) -> Result<(), Box<dyn Error>> {
    full_sync::save_materialized_paths(projector_dir, snapshot)?;
    full_sync::save_materialized_body_texts(projector_dir, snapshot)?;
    Ok(())
}

pub(super) fn load_saved_materialized_paths(
    projector_dir: &std::path::Path,
) -> std::collections::BTreeSet<(std::path::PathBuf, std::path::PathBuf)> {
    full_sync::load_materialized_paths(projector_dir).unwrap_or_default()
}
