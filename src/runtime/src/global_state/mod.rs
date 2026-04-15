/**
@module PROJECTOR.RUNTIME.GLOBAL_STATE
Persists machine-global projector home state for server profiles, repo registration, and daemon lifecycle so multiple repos can share one control plane.
*/
// @fileimplements PROJECTOR.RUNTIME.GLOBAL_STATE
mod daemon_state;
mod projector_home;
mod server_profiles;
mod sync_registry;

pub use daemon_state::{
    FileMachineDaemonStateStore, MachineDaemonState, is_process_running, terminate_process,
};
pub use projector_home::ProjectorHome;
pub use server_profiles::{FileServerProfileStore, ServerProfile, ServerProfileRegistry};
pub use sync_registry::{FileMachineSyncRegistryStore, MachineSyncRegistry, RegisteredRepo};
