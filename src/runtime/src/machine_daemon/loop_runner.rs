/**
@module PROJECTOR.RUNTIME.MACHINE_DAEMON_LOOP
Owns the main machine-daemon control loop, including heartbeats, refresh cadence, local debounce flushes, and remote sweep scheduling.
*/
// @fileimplements PROJECTOR.RUNTIME.MACHINE_DAEMON_LOOP
use std::collections::BTreeMap;
use std::error::Error;
use std::path::PathBuf;
use std::thread;
use std::time::Instant;

use crate::{
    FileMachineDaemonStateStore, FileMachineSyncRegistryStore, HttpTransport, ProjectorHome,
};

use super::{
    DaemonIntervals, MachineDaemonOptions, RepoRuntime,
    repo_runtimes::refresh_repo_runtimes,
    scheduling::{
        affected_target_indexes, all_target_indexes_except, run_target_indexes, union_indexes,
    },
};

pub fn run_machine_daemon(
    home: ProjectorHome,
    options: &MachineDaemonOptions,
) -> Result<(), Box<dyn Error>> {
    let daemon_state_store = FileMachineDaemonStateStore::new(home.clone());
    let repo_registry_store = FileMachineSyncRegistryStore::new(home.clone());
    let cycles = options.cycles.unwrap_or(usize::MAX);
    let pid = std::process::id();
    let intervals = DaemonIntervals::from_options(options);
    let mut runtimes = BTreeMap::<PathBuf, RepoRuntime>::new();
    let mut transport_cache = BTreeMap::<String, HttpTransport>::new();

    daemon_state_store.write_running(pid)?;
    let mut last_registry_refresh = Instant::now() - intervals.registry_refresh;
    let mut last_polling_backstop = Instant::now() - intervals.polling_backstop;
    let mut last_remote_sweep = Instant::now() - intervals.remote_sweep;

    for cycle in 0..cycles {
        daemon_state_store.heartbeat(pid)?;
        let now = Instant::now();

        if cycle == 0 || now.duration_since(last_registry_refresh) >= intervals.registry_refresh {
            refresh_repo_runtimes(&repo_registry_store, &home, &mut runtimes)?;
            last_registry_refresh = now;
        }

        let run_polling_backstop =
            cycle == 0 || now.duration_since(last_polling_backstop) >= intervals.polling_backstop;
        if run_polling_backstop {
            last_polling_backstop = now;
        }

        let run_remote_sweep =
            cycle == 0 || now.duration_since(last_remote_sweep) >= intervals.remote_sweep;
        if run_remote_sweep {
            last_remote_sweep = now;
        }

        for runtime in runtimes.values_mut() {
            let events = runtime.watcher.poll_with_backstop(run_polling_backstop)?;
            let changed_target_indexes = affected_target_indexes(&runtime.sync_targets, &events);
            runtime
                .pending_local_work
                .observe(&changed_target_indexes, now);
            let ready_local_indexes = runtime
                .pending_local_work
                .drain_if_ready(now, intervals.local_debounce);
            run_target_indexes(
                &runtime.sync_targets,
                &ready_local_indexes,
                options.poll_ms,
                &mut transport_cache,
            );

            if run_remote_sweep {
                let excluded_indexes = union_indexes(
                    &changed_target_indexes,
                    &runtime.pending_local_work.excludes(),
                );
                let remote_only_indexes =
                    all_target_indexes_except(runtime.sync_targets.len(), &excluded_indexes);
                run_target_indexes(
                    &runtime.sync_targets,
                    &remote_only_indexes,
                    options.poll_ms,
                    &mut transport_cache,
                );
            }
        }

        if cycle + 1 < cycles {
            thread::sleep(intervals.tick);
        }
    }

    daemon_state_store
        .clear()
        .or_else(super::scheduling::ignore_missing_file)?;
    Ok(())
}
