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
        registry.repos.retain(|repo| repo.repo_root != repo_root);
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
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_millis()
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use projector_domain::{ActorId, RepoSyncConfig, RepoSyncEntry, SyncEntryKind, WorkspaceId};

    use super::{FileMachineSyncRegistryStore, ProjectorHome};
    use crate::test_support::temp_projector_home;

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
            history_compaction_policies: vec![],
        }
    }

    #[test]
    fn sync_repo_registers_and_unregisters_repo() {
        let store =
            FileMachineSyncRegistryStore::new(ProjectorHome::new(temp_projector_home("registry")));
        let repo_root = Path::new("/tmp/projector-repo");

        let registry = store
            .sync_repo(repo_root, &sample_config())
            .expect("register repo");
        assert_eq!(registry.repos.len(), 1);
        assert_eq!(registry.repos[0].repo_root, repo_root);
        assert_eq!(registry.repos[0].entry_count, 2);
        assert_eq!(
            registry.repos[0].server_profiles,
            vec!["homebox".to_owned(), "workbox".to_owned()]
        );

        let registry = store
            .sync_repo(repo_root, &RepoSyncConfig::default())
            .expect("unregister repo");
        assert!(registry.repos.is_empty());
    }
}
