/**
@module PROJECTOR.RUNTIME.MACHINE_DAEMON_SCHEDULER
Owns target-index selection, watcher-event matching, per-target sync execution, and shared transport caching for the machine daemon.
*/
// @fileimplements PROJECTOR.RUNTIME.MACHINE_DAEMON_SCHEDULER
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::Path;

use projector_domain::{SyncContext, SyncEntryTarget};

use crate::{HttpTransport, SyncLoopOptions, SyncRunner, WatcherEvent};

pub(super) fn affected_target_indexes(
    sync_targets: &[SyncEntryTarget],
    events: &[WatcherEvent],
) -> Vec<usize> {
    let mut indexes = BTreeSet::new();
    for (index, target) in sync_targets.iter().enumerate() {
        if events
            .iter()
            .any(|event| sync_target_matches_event(target, watcher_event_path(event)))
        {
            indexes.insert(index);
        }
    }
    indexes.into_iter().collect()
}

fn sync_target_matches_event(sync_target: &SyncEntryTarget, event_path: &Path) -> bool {
    match sync_target.mount.kind {
        projector_domain::SyncEntryKind::Directory => {
            event_path == sync_target.mount.absolute_path
                || event_path.starts_with(&sync_target.mount.absolute_path)
        }
        projector_domain::SyncEntryKind::File => event_path == sync_target.mount.absolute_path,
    }
}

fn watcher_event_path(event: &WatcherEvent) -> &Path {
    match event {
        WatcherEvent::FileChanged(path)
        | WatcherEvent::FileCreated(path)
        | WatcherEvent::FileDeleted(path) => path.as_path(),
    }
}

pub(super) fn all_target_indexes_except(total: usize, excluded: &[usize]) -> Vec<usize> {
    let excluded = excluded.iter().copied().collect::<BTreeSet<_>>();
    (0..total)
        .filter(|index| !excluded.contains(index))
        .collect()
}

pub(super) fn union_indexes(left: &[usize], right: &[usize]) -> Vec<usize> {
    left.iter()
        .chain(right.iter())
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(super) fn run_target_indexes(
    sync_targets: &[SyncEntryTarget],
    indexes: &[usize],
    poll_ms: u64,
    transport_cache: &mut BTreeMap<String, HttpTransport>,
) {
    for index in indexes {
        if let Some(sync_target) = sync_targets.get(*index) {
            let transport = sync_target
                .server_addr()
                .map(|server_addr| cached_transport(transport_cache, server_addr));
            let mut runner = SyncRunner::new(sync_target, transport);
            let _ = runner.run(&SyncLoopOptions {
                watch: false,
                poll_ms,
                watch_cycles: None,
            });
        }
    }
}

pub(super) fn cached_transport(
    transport_cache: &mut BTreeMap<String, HttpTransport>,
    server_addr: &str,
) -> HttpTransport {
    transport_cache
        .entry(server_addr.to_owned())
        .or_insert_with(|| HttpTransport::new(format!("http://{server_addr}")))
        .clone()
}

pub(super) fn ignore_missing_file(err: io::Error) -> Result<(), io::Error> {
    if err.kind() == io::ErrorKind::NotFound {
        Ok(())
    } else {
        Err(err)
    }
}
