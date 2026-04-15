/**
@module PROJECTOR.RUNTIME.MACHINE_DAEMON
Coordinates the machine-global projector sync loop by delegating daemon scheduling, repo-runtime refresh, and targeted sync execution to narrower runtime modules.
*/
// @fileimplements PROJECTOR.RUNTIME.MACHINE_DAEMON
use std::collections::BTreeSet;
use std::io;
use std::time::{Duration, Instant};

use projector_domain::SyncEntryTarget;

use crate::RuntimeWatcher;

mod loop_runner;
mod repo_runtimes;
mod scheduling;

pub use loop_runner::run_machine_daemon;
use repo_runtimes::watched_mounts;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MachineDaemonOptions {
    pub poll_ms: u64,
    pub cycles: Option<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DaemonIntervals {
    pub(super) tick: Duration,
    pub(super) local_debounce: Duration,
    pub(super) registry_refresh: Duration,
    pub(super) polling_backstop: Duration,
    pub(super) remote_sweep: Duration,
}

impl DaemonIntervals {
    fn from_options(options: &MachineDaemonOptions) -> Self {
        let tick = Duration::from_millis(options.poll_ms.max(25));
        let tick_ms = tick.as_millis() as u64;
        Self {
            tick,
            local_debounce: Duration::from_millis((tick_ms * 3).max(200)),
            registry_refresh: Duration::from_millis((tick_ms * 4).max(1_000)),
            polling_backstop: Duration::from_millis((tick_ms * 20).max(5_000)),
            remote_sweep: Duration::from_millis((tick_ms * 40).max(15_000)),
        }
    }
}

pub(super) struct RepoRuntime {
    pub(super) sync_targets: Vec<SyncEntryTarget>,
    pub(super) watcher: RuntimeWatcher,
    pub(super) pending_local_work: PendingLocalWork,
}

impl RepoRuntime {
    pub(super) fn new(sync_targets: Vec<SyncEntryTarget>) -> Result<Self, io::Error> {
        let watcher = RuntimeWatcher::new(watched_mounts(&sync_targets))?;
        Ok(Self {
            sync_targets,
            watcher,
            pending_local_work: PendingLocalWork::default(),
        })
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct PendingLocalWork {
    indexes: BTreeSet<usize>,
    last_event_at: Option<Instant>,
}

impl PendingLocalWork {
    pub(super) fn observe(&mut self, indexes: &[usize], now: Instant) {
        if indexes.is_empty() {
            return;
        }
        self.indexes.extend(indexes.iter().copied());
        self.last_event_at = Some(now);
    }

    pub(super) fn excludes(&self) -> Vec<usize> {
        self.indexes.iter().copied().collect()
    }

    pub(super) fn drain_if_ready(&mut self, now: Instant, debounce: Duration) -> Vec<usize> {
        let Some(last_event_at) = self.last_event_at else {
            return Vec::new();
        };
        if now.duration_since(last_event_at) < debounce {
            return Vec::new();
        }
        let ready = self.indexes.iter().copied().collect::<Vec<_>>();
        self.indexes.clear();
        self.last_event_at = None;
        ready
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use crate::WatcherEvent;
    use projector_domain::{ActorId, ProjectionMount, SyncEntryKind, WorkspaceId};

    use super::{
        DaemonIntervals, MachineDaemonOptions, PendingLocalWork, SyncEntryTarget,
        scheduling::{
            affected_target_indexes, all_target_indexes_except, cached_transport, union_indexes,
        },
    };

    fn target(name: &str, path: &str, kind: SyncEntryKind) -> SyncEntryTarget {
        SyncEntryTarget {
            entry_id: format!("entry-{name}"),
            workspace_id: WorkspaceId::new(format!("ws-{name}")),
            actor_id: ActorId::new(format!("actor-{name}")),
            server_addr: Some("127.0.0.1:1".to_owned()),
            projector_dir: PathBuf::from("/tmp/projector"),
            source_repo_name: Some("repo".to_owned()),
            mount: ProjectionMount {
                relative_path: PathBuf::from(path),
                absolute_path: PathBuf::from("/repo").join(path),
                kind,
            },
        }
    }

    // @verifies PROJECTOR.CLI.SYNC.USES_SLOW_BACKSTOPS
    #[test]
    fn daemon_intervals_back_off_from_the_main_tick() {
        let intervals = DaemonIntervals::from_options(&MachineDaemonOptions {
            poll_ms: 250,
            cycles: None,
        });

        assert!(intervals.registry_refresh > intervals.tick);
        assert!(intervals.polling_backstop > intervals.tick);
        assert!(intervals.remote_sweep > intervals.polling_backstop);
        assert!(intervals.local_debounce > intervals.tick);
    }

    // @verifies PROJECTOR.CLI.SYNC.SCOPES_LOCAL_WORK_TO_CHANGED_ENTRIES
    #[test]
    fn affected_target_indexes_scope_local_work_to_changed_entries() {
        let sync_targets = vec![
            target("private", "private", SyncEntryKind::Directory),
            target("notes", "notes/todo.txt", SyncEntryKind::File),
        ];
        let events = vec![
            WatcherEvent::FileChanged(PathBuf::from("/repo/private/briefs/live.md")),
            WatcherEvent::FileChanged(PathBuf::from("/repo/other/ignored.md")),
        ];

        assert_eq!(affected_target_indexes(&sync_targets, &events), vec![0]);
    }

    #[test]
    fn remote_sweep_indexes_skip_targets_already_synced_from_local_events() {
        assert_eq!(all_target_indexes_except(4, &[1, 3]), vec![0, 2]);
    }

    // @verifies PROJECTOR.CLI.SYNC.COALESCES_LOCAL_EVENT_BURSTS
    #[test]
    fn pending_local_work_coalesces_bursts_before_flushing_one_batch() {
        let debounce = Duration::from_millis(200);
        let start = Instant::now();
        let mut pending = PendingLocalWork::default();

        pending.observe(&[0, 1], start);
        pending.observe(&[1, 2], start + Duration::from_millis(50));

        assert_eq!(
            pending.drain_if_ready(start + Duration::from_millis(150), debounce),
            Vec::<usize>::new()
        );
        assert_eq!(
            pending.drain_if_ready(start + Duration::from_millis(260), debounce),
            vec![0, 1, 2]
        );
    }

    // @verifies PROJECTOR.CLI.SYNC.REUSES_SERVER_TRANSPORTS
    #[test]
    fn cached_transport_reuses_one_transport_per_server_address() {
        let mut cache = std::collections::BTreeMap::new();

        let _first = cached_transport(&mut cache, "127.0.0.1:9001");
        let _second = cached_transport(&mut cache, "127.0.0.1:9001");
        let _third = cached_transport(&mut cache, "127.0.0.1:9002");

        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn union_indexes_deduplicates_changed_and_pending_sets() {
        assert_eq!(union_indexes(&[0, 2], &[2, 3]), vec![0, 2, 3]);
    }
}
