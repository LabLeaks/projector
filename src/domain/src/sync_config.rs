/**
@module PROJECTOR.DOMAIN.SYNC_CONFIG
Defines repo-local path-scoped sync-entry configuration types used to replace one coarse checkout binding with explicit synced files and folders.
*/
// @fileimplements PROJECTOR.DOMAIN.SYNC_CONFIG
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{ActorId, WorkspaceId};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SyncEntryKind {
    File,
    Directory,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RepoSyncEntry {
    pub entry_id: String,
    pub workspace_id: WorkspaceId,
    pub actor_id: ActorId,
    pub server_profile_id: String,
    pub local_relative_path: PathBuf,
    pub remote_relative_path: PathBuf,
    pub kind: SyncEntryKind,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RepoSyncConfig {
    pub entries: Vec<RepoSyncEntry>,
}
