/**
@module PROJECTOR.RUNTIME.MACHINE_SYNC_REGISTRY
Owns the machine-global registry of repos that currently have repo-local sync entries so the daemon can discover active work without scanning the filesystem.
*/
// @fileimplements PROJECTOR.RUNTIME.MACHINE_SYNC_REGISTRY
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use projector_domain::RepoSyncConfig;
use serde::{Deserialize, Serialize};

use super::ProjectorHome;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct MachineSyncRegistry {
    pub repos: Vec<RegisteredRepo>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RegisteredRepo {
    pub repo_root: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_identity: Option<String>,
    pub entry_count: usize,
    pub server_profiles: Vec<String>,
    pub updated_at_ms: u128,
}

#[derive(Clone, Debug)]
pub struct FileMachineSyncRegistryStore {
    home: ProjectorHome,
}

impl FileMachineSyncRegistryStore {
    pub fn new(home: ProjectorHome) -> Self {
        Self { home }
    }

    pub fn load(&self) -> Result<MachineSyncRegistry, io::Error> {
        let path = self.home.repo_registry_path();
        if !path.exists() {
            return Ok(MachineSyncRegistry::default());
        }

        let content = fs::read_to_string(path)?;
        serde_json::from_str(&content).map_err(|err| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("machine sync registry is invalid JSON: {err}"),
            )
        })
    }

    pub fn save(&self, registry: &MachineSyncRegistry) -> Result<(), io::Error> {
        self.home.ensure_root()?;
        let content = serde_json::to_string_pretty(registry).map_err(|err| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("failed to encode machine sync registry: {err}"),
            )
        })?;
        fs::write(self.home.repo_registry_path(), content)
    }

    pub fn sync_repo(
        &self,
        repo_root: &Path,
        config: &RepoSyncConfig,
    ) -> Result<MachineSyncRegistry, io::Error> {
        let mut registry = self.load()?;
        let repo_identity = if config.entries.is_empty() {
            load_repo_identity(repo_root)?
        } else {
            Some(ensure_repo_identity(repo_root)?)
        };
        registry.repos.retain(|repo| {
            !same_repo_root(&repo.repo_root, repo_root)
                && !same_repo_identity(repo.repo_identity.as_deref(), repo_identity.as_deref())
        });
        if !config.entries.is_empty() {
            let mut server_profiles = config
                .entries
                .iter()
                .map(|entry| entry.server_profile_id.clone())
                .collect::<Vec<_>>();
            server_profiles.sort();
            server_profiles.dedup();
            registry.repos.push(RegisteredRepo {
                repo_root: repo_root.to_path_buf(),
                repo_identity,
                entry_count: config.entries.len(),
                server_profiles,
                updated_at_ms: now_ms(),
            });
        }
        registry
            .repos
            .sort_by(|left, right| left.repo_root.cmp(&right.repo_root));
        self.save(&registry)?;
        Ok(registry)
    }

    pub fn unregister_repo(&self, repo_root: &Path) -> Result<MachineSyncRegistry, io::Error> {
        let mut registry = self.load()?;
        let repo_identity = load_repo_identity(repo_root)?;
        registry.repos.retain(|repo| {
            !same_repo_root(&repo.repo_root, repo_root)
                && !same_repo_identity(repo.repo_identity.as_deref(), repo_identity.as_deref())
        });
        self.save(&registry)?;
        Ok(registry)
    }

    pub fn repo_is_registered(&self, repo_root: &Path) -> Result<bool, io::Error> {
        let registry = self.load()?;
        let repo_identity = load_repo_identity(repo_root)?;
        Ok(registry.repos.iter().any(|repo| {
            same_repo_root(&repo.repo_root, repo_root)
                || same_repo_identity(repo.repo_identity.as_deref(), repo_identity.as_deref())
        }))
    }
}

fn same_repo_root(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    match (fs::canonicalize(left), fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => normalize_path_lexically(left) == normalize_path_lexically(right),
    }
}

fn same_repo_identity(left: Option<&str>, right: Option<&str>) -> bool {
    matches!((left, right), (Some(left), Some(right)) if left == right)
}

fn repo_identity_path(repo_root: &Path) -> PathBuf {
    repo_root.join(".projector/repo-identity")
}

fn ensure_repo_identity(repo_root: &Path) -> Result<String, io::Error> {
    if let Some(identity) = load_repo_identity(repo_root)? {
        return Ok(identity);
    }
    let projector_dir = repo_root.join(".projector");
    fs::create_dir_all(&projector_dir)?;
    let identity = format!("repo-{}-{}", now_ns(), std::process::id());
    fs::write(repo_identity_path(repo_root), format!("{identity}\n"))?;
    Ok(identity)
}

fn load_repo_identity(repo_root: &Path) -> Result<Option<String>, io::Error> {
    let path = repo_identity_path(repo_root);
    if !path.exists() {
        return Ok(None);
    }
    let identity = fs::read_to_string(path)?.trim().to_owned();
    if identity.is_empty() {
        return Ok(None);
    }
    Ok(Some(identity))
}

fn normalize_path_lexically(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_millis()
}

fn now_ns() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use projector_domain::{ActorId, RepoSyncConfig, RepoSyncEntry, SyncEntryKind, WorkspaceId};

    use super::{FileMachineSyncRegistryStore, ProjectorHome};
    use crate::test_support::{temp_projector_home, temp_repo_root};

    fn sample_config() -> RepoSyncConfig {
        RepoSyncConfig {
            entries: vec![
                RepoSyncEntry {
                    entry_id: "entry-private".to_owned(),
                    workspace_id: WorkspaceId::new("ws-sample"),
                    actor_id: ActorId::new("actor-sample"),
                    server_profile_id: "homebox".to_owned(),
                    local_relative_path: PathBuf::from("private"),
                    remote_relative_path: PathBuf::from("private"),
                    kind: SyncEntryKind::Directory,
                },
                RepoSyncEntry {
                    entry_id: "entry-notes".to_owned(),
                    workspace_id: WorkspaceId::new("ws-sample"),
                    actor_id: ActorId::new("actor-sample"),
                    server_profile_id: "workbox".to_owned(),
                    local_relative_path: PathBuf::from("notes/index.html"),
                    remote_relative_path: PathBuf::from("notes/index.html"),
                    kind: SyncEntryKind::File,
                },
            ],
        }
    }

    #[test]
    fn sync_repo_registers_and_unregisters_repo() {
        let store =
            FileMachineSyncRegistryStore::new(ProjectorHome::new(temp_projector_home("registry")));
        let repo_root = temp_repo_root("registry-repo");

        let registry = store
            .sync_repo(&repo_root, &sample_config())
            .expect("register repo");
        assert_eq!(registry.repos.len(), 1);
        assert_eq!(registry.repos[0].repo_root, repo_root);
        assert!(registry.repos[0].repo_identity.is_some());
        assert_eq!(registry.repos[0].entry_count, 2);
        assert_eq!(
            registry.repos[0].server_profiles,
            vec!["homebox".to_owned(), "workbox".to_owned()]
        );

        let registry = store
            .sync_repo(&repo_root, &RepoSyncConfig::default())
            .expect("unregister repo");
        assert!(registry.repos.is_empty());
    }

    #[test]
    fn unregister_repo_removes_only_that_repo() {
        let store = FileMachineSyncRegistryStore::new(ProjectorHome::new(temp_projector_home(
            "registry-unregister",
        )));
        let repo_root = temp_repo_root("registry-unregister-repo");
        let other_repo_root = temp_repo_root("registry-unregister-other-repo");

        store
            .sync_repo(&repo_root, &sample_config())
            .expect("register repo");
        store
            .sync_repo(&other_repo_root, &sample_config())
            .expect("register other repo");

        let registry = store.unregister_repo(&repo_root).expect("unregister repo");
        assert_eq!(registry.repos.len(), 1);
        assert_eq!(registry.repos[0].repo_root, other_repo_root);
    }

    #[test]
    fn sync_repo_follows_moved_repo_identity_to_new_root() {
        let store = FileMachineSyncRegistryStore::new(ProjectorHome::new(temp_projector_home(
            "registry-moved-root",
        )));
        let old_repo_root = temp_repo_root("registry-moved-old");
        let new_repo_root = old_repo_root.with_file_name(format!(
            "{}-new",
            old_repo_root
                .file_name()
                .expect("old repo has filename")
                .to_string_lossy()
        ));

        store
            .sync_repo(&old_repo_root, &sample_config())
            .expect("register old root");
        std::fs::rename(&old_repo_root, &new_repo_root).expect("move repo root");

        let registry = store
            .sync_repo(&new_repo_root, &sample_config())
            .expect("register moved root");

        assert_eq!(registry.repos.len(), 1);
        assert_eq!(registry.repos[0].repo_root, new_repo_root);
        assert!(
            store
                .repo_is_registered(&registry.repos[0].repo_root)
                .expect("repo registered")
        );
    }
}
