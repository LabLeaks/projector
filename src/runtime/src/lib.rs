pub mod binding_store;
pub mod daemon;
pub mod global_state;
pub mod lease;
pub mod machine_daemon;
pub mod materializer;
pub mod provenance;
pub mod status;
pub mod sync_config_store;
pub mod sync_issue;
pub mod sync_targets;
#[cfg(test)]
pub(crate) mod test_support;
pub mod transport;
pub mod watcher;

pub use binding_store::{BindingStore, FileBindingStore, discover_repo_root};
pub use daemon::{
    DaemonEvent, SyncLoopOptions, SyncRunReport, SyncRunner, apply_authoritative_snapshot,
};
pub use global_state::{
    FileMachineDaemonStateStore, FileMachineSyncRegistryStore, FileServerProfileStore,
    MachineDaemonState, MachineSyncRegistry, ProjectorHome, RegisteredRepo, ServerProfile,
    ServerProfileRegistry, is_process_running, terminate_process,
};
pub use lease::{ActiveRuntimeLease, FileRuntimeLeaseStore};
pub use machine_daemon::{MachineDaemonOptions, run_machine_daemon};
pub use materializer::{MaterializationPlan, Materializer, ProjectionMaterializer};
pub use provenance::{FileProvenanceLog, StoredEvent};
pub use status::{FileRuntimeStatusStore, RuntimeStatus};
pub use sync_config_store::{
    FileRepoSyncConfigStore, HistoryCompactionPolicySource, ResolvedHistoryCompactionPolicy,
};
pub use sync_issue::{SyncIssue, SyncIssueDisposition, classify_sync_issue};
pub use sync_targets::{derive_sync_targets, load_sync_targets};
pub use transport::{HttpTransport, Transport};
pub use watcher::{
    NotifyWatcher, PollingWatcher, RuntimeWatcher, WatchedMount, Watcher, WatcherEvent,
};
