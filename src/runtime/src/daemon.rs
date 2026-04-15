/**
@module PROJECTOR.RUNTIME.DAEMON
Owns the top-level sync loop and watch-loop orchestration while delegating reconciliation and recovery details to narrower runtime modules.
*/
// @fileimplements PROJECTOR.RUNTIME.DAEMON
use std::error::Error;
use std::io;
use std::thread;
use std::time::Duration;

use projector_domain::{BootstrapSnapshot, SyncContext};

use crate::{
    FileProvenanceLog, FileRuntimeLeaseStore, FileRuntimeStatusStore, HttpTransport, Materializer,
    ProjectionMaterializer, Transport, Watcher,
};

mod reconcile;
mod recovery;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DaemonEvent {
    SnapshotReceived(BootstrapSnapshot),
    LocalFilesystem(crate::WatcherEvent),
    ShutdownRequested,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyncLoopOptions {
    pub watch: bool,
    pub poll_ms: u64,
    pub watch_cycles: Option<usize>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SyncRunReport {
    pub snapshot: BootstrapSnapshot,
    pub cursor: u64,
}

pub struct SyncRunner<'a, C, T> {
    binding: &'a C,
    materializer: ProjectionMaterializer,
    lease_store: FileRuntimeLeaseStore,
    status_store: FileRuntimeStatusStore,
    log: FileProvenanceLog,
    transport: Option<T>,
}

impl<'a, C> SyncRunner<'a, C, HttpTransport>
where
    C: SyncContext,
{
    pub fn connect(binding: &'a C) -> Self {
        let transport = binding
            .server_addr()
            .map(|server_addr| HttpTransport::new(format!("http://{server_addr}")));
        Self::new(binding, transport)
    }
}

impl<'a, C, T> SyncRunner<'a, C, T>
where
    C: SyncContext,
{
    pub fn new(binding: &'a C, transport: Option<T>) -> Self {
        Self {
            binding,
            materializer: ProjectionMaterializer::new(binding),
            lease_store: FileRuntimeLeaseStore::new(binding.projector_dir().join("runtime.lock")),
            status_store: FileRuntimeStatusStore::new(binding.projector_dir().join("status.txt")),
            log: FileProvenanceLog::new(binding.projector_dir().join("events.log")),
            transport,
        }
    }
}

pub fn apply_authoritative_snapshot(
    binding: &dyn SyncContext,
    snapshot: &BootstrapSnapshot,
) -> Result<(), Box<dyn Error>> {
    let materializer = ProjectionMaterializer::new(binding);
    let known_mounts = binding
        .projection_mounts()
        .into_iter()
        .map(|mount| mount.relative_path)
        .collect::<std::collections::BTreeSet<_>>();
    materializer.ensure_projection_roots()?;
    let mut plan = materializer.plan(snapshot)?;
    let previous_paths = reconcile::load_saved_materialized_paths(binding.projector_dir());
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
        plan.files_to_remove
            .push(materializer.resolve_projection_path(&mount_relative_path, &relative_path)?);
    }
    plan.files_to_remove.sort();
    plan.files_to_remove.dedup();
    materializer.apply(&plan)?;
    reconcile::save_snapshot_checkpoints(binding.projector_dir(), snapshot)?;
    Ok(())
}

impl<C, T> SyncRunner<'_, C, T>
where
    C: SyncContext,
    T: Transport<Error = io::Error>,
{
    pub fn run(&mut self, options: &SyncLoopOptions) -> Result<SyncRunReport, Box<dyn Error>> {
        const MAX_IMMEDIATE_RETRIES: usize = 2;

        let _runtime_lease = if options.watch {
            Some(self.lease_store.acquire()?)
        } else {
            None
        };

        self.reset_run_status()?;

        let mut immediate_retry_attempts = 0usize;
        let mut rebootstrap_retry_used = false;
        let mut recovery_attempt_count = 0usize;
        let mut last_recovery_action = None::<String>;

        loop {
            match self.run_inner(options) {
                Ok(report) => {
                    self.persist_recovery_status(
                        recovery_attempt_count,
                        last_recovery_action.clone(),
                    )?;
                    return Ok(report);
                }
                Err(err) => {
                    let issue = crate::classify_sync_issue(err.as_ref());
                    match issue.disposition {
                        crate::SyncIssueDisposition::RetryImmediately
                            if immediate_retry_attempts < MAX_IMMEDIATE_RETRIES =>
                        {
                            immediate_retry_attempts += 1;
                            recovery_attempt_count += 1;
                            last_recovery_action = Some("retry_immediately".to_owned());
                            self.record_recovery_action(
                                "retry_immediately",
                                recovery_attempt_count,
                            )?;
                            self.persist_recovery_status(
                                recovery_attempt_count,
                                last_recovery_action.clone(),
                            )?;
                            thread::sleep(Duration::from_millis(
                                50 * immediate_retry_attempts as u64,
                            ));
                        }
                        crate::SyncIssueDisposition::NeedsRebootstrap
                            if !rebootstrap_retry_used =>
                        {
                            rebootstrap_retry_used = true;
                            recovery_attempt_count += 1;
                            last_recovery_action = Some("needs_rebootstrap_retry".to_owned());
                            self.record_recovery_action(
                                "needs_rebootstrap_retry",
                                recovery_attempt_count,
                            )?;
                            self.persist_recovery_status(
                                recovery_attempt_count,
                                last_recovery_action.clone(),
                            )?;
                        }
                        _ => {
                            self.record_sync_issue(
                                err.as_ref(),
                                recovery_attempt_count,
                                last_recovery_action.clone(),
                            )?;
                            return Err(err);
                        }
                    }
                }
            }
        }
    }

    fn run_inner(&mut self, options: &SyncLoopOptions) -> Result<SyncRunReport, Box<dyn Error>> {
        let watch_cycles = if options.watch {
            options.watch_cycles.unwrap_or(usize::MAX)
        } else {
            1
        };

        let (mut final_snapshot, mut cursor) = self.reconcile_snapshot()?;
        self.apply_snapshot(&final_snapshot)?;

        if !options.watch {
            self.save_status(false, 0, Some(recovery::now_ms()), 0, None, 0, None)?;
            return Ok(SyncRunReport {
                snapshot: final_snapshot,
                cursor,
            });
        }

        let mut watcher = crate::RuntimeWatcher::new(
            self.binding
                .projection_mounts()
                .into_iter()
                .map(|mount| crate::WatchedMount {
                    absolute_path: mount.absolute_path,
                    kind: mount.kind,
                })
                .collect(),
        )?;
        self.save_status(
            watch_cycles > 1,
            0,
            Some(recovery::now_ms()),
            0,
            None,
            0,
            None,
        )?;

        for cycle in 1..watch_cycles {
            thread::sleep(Duration::from_millis(options.poll_ms));
            let pending_events = watcher.poll()?;

            if !pending_events.is_empty() {
                self.save_status(
                    true,
                    pending_events.len(),
                    Some(recovery::now_ms()),
                    0,
                    None,
                    0,
                    None,
                )?;
                if let Some((updated_snapshot, updated_cursor)) =
                    self.push_watcher_events(&final_snapshot, cursor, &pending_events)?
                {
                    self.apply_snapshot(&updated_snapshot)?;
                    final_snapshot = updated_snapshot;
                    cursor = updated_cursor;
                } else if let Some((remote_snapshot, updated_cursor)) =
                    self.pull_remote_snapshot_if_changed(cursor, &final_snapshot)?
                {
                    self.apply_snapshot(&remote_snapshot)?;
                    final_snapshot = remote_snapshot;
                    cursor = updated_cursor;
                }
            } else if let Some((remote_snapshot, updated_cursor)) =
                self.pull_remote_snapshot_if_changed(cursor, &final_snapshot)?
            {
                self.apply_snapshot(&remote_snapshot)?;
                final_snapshot = remote_snapshot;
                cursor = updated_cursor;
            }

            self.save_status(
                cycle + 1 < watch_cycles,
                0,
                Some(recovery::now_ms()),
                0,
                None,
                0,
                None,
            )?;
        }

        Ok(SyncRunReport {
            snapshot: final_snapshot,
            cursor,
        })
    }
}
