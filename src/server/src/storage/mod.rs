/**
@module PROJECTOR.SERVER.STORAGE
Coordinates the store contract, error model, and backend store implementations over the narrower server workspace, manifest, body, provenance, history, and restore modules.
*/
// @fileimplements PROJECTOR.SERVER.STORAGE
mod bodies;
mod body_projection;
mod body_persistence;
mod body_state;
mod contract;
mod error;
mod file_store;
mod history;
mod manifest;
mod postgres_store;
mod provenance;
mod sqlite;
mod workspaces;

pub use contract::WorkspaceStore;
pub use error::StoreError;
pub use file_store::{FileWorkspaceStore, write_workspace_snapshot};
pub use postgres_store::PostgresWorkspaceStore;
pub use sqlite::SqliteWorkspaceStore;

use std::path::{Path, PathBuf};

pub(crate) fn state_workspaces_root(state_dir: &Path) -> PathBuf {
    state_dir.join("workspaces")
}
