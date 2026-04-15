/**
@module PROJECTOR.DOMAIN.BINDING
Defines checkout binding and projection-root types shared across the CLI, runtime, and server boundaries.
*/
// @fileimplements PROJECTOR.DOMAIN.BINDING
use std::path::{Path, PathBuf};

use crate::{ActorId, SyncEntryKind, WorkspaceId};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectionRoots {
    pub projector_dir: PathBuf,
    pub projection_paths: Vec<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckoutBinding {
    pub workspace_id: WorkspaceId,
    pub actor_id: ActorId,
    pub projection_relative_paths: Vec<PathBuf>,
    pub projection_kinds: Vec<SyncEntryKind>,
    pub server_addr: Option<String>,
    pub roots: ProjectionRoots,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectionMount {
    pub relative_path: PathBuf,
    pub absolute_path: PathBuf,
    pub kind: SyncEntryKind,
}

pub trait SyncContext {
    fn workspace_id(&self) -> &WorkspaceId;
    fn actor_id(&self) -> &ActorId;
    fn server_addr(&self) -> Option<&str>;
    fn projector_dir(&self) -> &Path;
    fn projection_mounts(&self) -> Vec<ProjectionMount>;
    fn source_repo_name(&self) -> Option<&str> {
        None
    }
    fn sync_entry_kind(&self) -> Option<SyncEntryKind> {
        None
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyncEntryTarget {
    pub entry_id: String,
    pub workspace_id: WorkspaceId,
    pub actor_id: ActorId,
    pub server_addr: Option<String>,
    pub projector_dir: PathBuf,
    pub source_repo_name: Option<String>,
    pub mount: ProjectionMount,
}

impl CheckoutBinding {
    pub fn projection_mounts(&self) -> Vec<ProjectionMount> {
        self.projection_relative_paths
            .iter()
            .cloned()
            .zip(self.projection_kinds.iter().cloned())
            .zip(self.roots.projection_paths.iter().cloned())
            .map(|((relative_path, kind), absolute_path)| ProjectionMount {
                relative_path,
                absolute_path,
                kind,
            })
            .collect()
    }
}

impl SyncContext for CheckoutBinding {
    fn workspace_id(&self) -> &WorkspaceId {
        &self.workspace_id
    }

    fn actor_id(&self) -> &ActorId {
        &self.actor_id
    }

    fn server_addr(&self) -> Option<&str> {
        self.server_addr.as_deref()
    }

    fn projector_dir(&self) -> &Path {
        &self.roots.projector_dir
    }

    fn projection_mounts(&self) -> Vec<ProjectionMount> {
        self.projection_mounts()
    }

    fn source_repo_name(&self) -> Option<&str> {
        self.roots
            .projector_dir
            .parent()
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
    }

    fn sync_entry_kind(&self) -> Option<SyncEntryKind> {
        if self.projection_kinds.len() == 1 {
            Some(self.projection_kinds[0].clone())
        } else {
            None
        }
    }
}

impl SyncContext for SyncEntryTarget {
    fn workspace_id(&self) -> &WorkspaceId {
        &self.workspace_id
    }

    fn actor_id(&self) -> &ActorId {
        &self.actor_id
    }

    fn server_addr(&self) -> Option<&str> {
        self.server_addr.as_deref()
    }

    fn projector_dir(&self) -> &Path {
        &self.projector_dir
    }

    fn projection_mounts(&self) -> Vec<ProjectionMount> {
        vec![self.mount.clone()]
    }

    fn source_repo_name(&self) -> Option<&str> {
        self.source_repo_name.as_deref()
    }

    fn sync_entry_kind(&self) -> Option<SyncEntryKind> {
        Some(self.mount.kind.clone())
    }
}
