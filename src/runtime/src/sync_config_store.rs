/**
@module PROJECTOR.RUNTIME.SYNC_CONFIG
Persists repo-local path-scoped sync-entry configuration under `.projector/` so the runtime can migrate away from one coarse checkout binding.
*/
// @fileimplements PROJECTOR.RUNTIME.SYNC_CONFIG
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use projector_domain::{
    HistoryCompactionPolicy, HistoryCompactionPolicyOverride, RepoSyncConfig, RepoSyncEntry,
};

const DEFAULT_HISTORY_COMPACTION_REVISIONS: usize = 100;
const DEFAULT_HISTORY_COMPACTION_FREQUENCY: usize = 10;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedHistoryCompactionPolicy {
    pub policy: HistoryCompactionPolicy,
    pub source: HistoryCompactionPolicySource,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HistoryCompactionPolicySource {
    Default,
    PathOverride { repo_relative_path: PathBuf },
    AncestorOverride { repo_relative_path: PathBuf },
}

#[derive(Clone, Debug)]
pub struct FileRepoSyncConfigStore {
    repo_root: PathBuf,
}

impl FileRepoSyncConfigStore {
    pub fn new(repo_root: impl Into<PathBuf>) -> Self {
        Self {
            repo_root: repo_root.into(),
        }
    }

    fn projector_dir(&self) -> PathBuf {
        self.repo_root.join(".projector")
    }

    fn config_path(&self) -> PathBuf {
        self.projector_dir().join("sync-entries.json")
    }

    pub fn load(&self) -> Result<RepoSyncConfig, io::Error> {
        let path = self.config_path();
        if !path.exists() {
            return Ok(RepoSyncConfig::default());
        }

        let content = fs::read_to_string(path)?;
        serde_json::from_str(&content).map_err(|err| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("sync-entry config is invalid JSON: {err}"),
            )
        })
    }

    pub fn save(&self, config: &RepoSyncConfig) -> Result<(), io::Error> {
        fs::create_dir_all(self.projector_dir())?;
        let content = serde_json::to_string_pretty(config).map_err(|err| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("failed to encode sync-entry config: {err}"),
            )
        })?;
        fs::write(self.config_path(), content)
    }

    pub fn upsert_entry(&self, entry: RepoSyncEntry) -> Result<(), io::Error> {
        let mut config = self.load()?;
        if let Some(existing) = config
            .entries
            .iter_mut()
            .find(|existing| existing.local_relative_path == entry.local_relative_path)
        {
            *existing = entry;
        } else {
            config.entries.push(entry);
        }
        config
            .entries
            .sort_by(|left, right| left.local_relative_path.cmp(&right.local_relative_path));
        self.save(&config)
    }

    pub fn remove_entry(&self, local_relative_path: &Path) -> Result<bool, io::Error> {
        let mut config = self.load()?;
        let original_len = config.entries.len();
        config
            .entries
            .retain(|entry| entry.local_relative_path != local_relative_path);
        let removed = config.entries.len() != original_len;
        if removed {
            self.save(&config)?;
        }
        Ok(removed)
    }

    pub fn resolve_history_compaction_policy(
        &self,
        repo_relative_path: &Path,
    ) -> Result<ResolvedHistoryCompactionPolicy, io::Error> {
        let config = self.load()?;
        let mut candidate = Some(repo_relative_path);
        while let Some(path) = candidate {
            if let Some(override_entry) = config
                .history_compaction_policies
                .iter()
                .find(|entry| entry.repo_relative_path == path)
            {
                let source = if path == repo_relative_path {
                    HistoryCompactionPolicySource::PathOverride {
                        repo_relative_path: path.to_path_buf(),
                    }
                } else {
                    HistoryCompactionPolicySource::AncestorOverride {
                        repo_relative_path: path.to_path_buf(),
                    }
                };
                return Ok(ResolvedHistoryCompactionPolicy {
                    policy: override_entry.policy.clone(),
                    source,
                });
            }
            candidate = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty());
        }

        Ok(ResolvedHistoryCompactionPolicy {
            policy: HistoryCompactionPolicy {
                revisions: DEFAULT_HISTORY_COMPACTION_REVISIONS,
                frequency: DEFAULT_HISTORY_COMPACTION_FREQUENCY,
            },
            source: HistoryCompactionPolicySource::Default,
        })
    }

    pub fn upsert_history_compaction_policy(
        &self,
        repo_relative_path: &Path,
        policy: HistoryCompactionPolicy,
    ) -> Result<(), io::Error> {
        let mut config = self.load()?;
        if let Some(existing) = config
            .history_compaction_policies
            .iter_mut()
            .find(|existing| existing.repo_relative_path == repo_relative_path)
        {
            existing.policy = policy;
        } else {
            config
                .history_compaction_policies
                .push(HistoryCompactionPolicyOverride {
                    repo_relative_path: repo_relative_path.to_path_buf(),
                    policy,
                });
        }
        config
            .history_compaction_policies
            .sort_by(|left, right| left.repo_relative_path.cmp(&right.repo_relative_path));
        self.save(&config)
    }

    pub fn remove_history_compaction_policy(
        &self,
        repo_relative_path: &Path,
    ) -> Result<bool, io::Error> {
        let mut config = self.load()?;
        let original_len = config.history_compaction_policies.len();
        config
            .history_compaction_policies
            .retain(|entry| entry.repo_relative_path != repo_relative_path);
        let removed = config.history_compaction_policies.len() != original_len;
        if removed {
            self.save(&config)?;
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::path::PathBuf;

    use projector_domain::{
        ActorId, HistoryCompactionPolicy, RepoSyncConfig, RepoSyncEntry, SyncEntryKind, WorkspaceId,
    };

    use super::{
        FileRepoSyncConfigStore, HistoryCompactionPolicySource, ResolvedHistoryCompactionPolicy,
    };
    use crate::test_support::temp_repo_root;

    fn sample_entry(local_relative_path: &str, kind: SyncEntryKind) -> RepoSyncEntry {
        RepoSyncEntry {
            entry_id: format!("entry-{local_relative_path}"),
            workspace_id: WorkspaceId::new("ws-sample"),
            actor_id: ActorId::new("actor-sample"),
            server_profile_id: "homebox".to_owned(),
            local_relative_path: PathBuf::from(local_relative_path),
            remote_relative_path: PathBuf::from(local_relative_path),
            kind,
        }
    }

    #[test]
    fn load_defaults_to_empty_config_when_missing() {
        let repo = temp_repo_root("sync-config-empty");
        let store = FileRepoSyncConfigStore::new(&repo);

        assert_eq!(
            store.load().expect("load config"),
            RepoSyncConfig::default()
        );
    }

    #[test]
    fn save_and_load_round_trip_sync_entries() {
        let repo = temp_repo_root("sync-config-roundtrip");
        let store = FileRepoSyncConfigStore::new(&repo);
        let config = RepoSyncConfig {
            entries: vec![
                sample_entry("private", SyncEntryKind::Directory),
                sample_entry("notes/today.html", SyncEntryKind::File),
            ],
            history_compaction_policies: vec![],
        };

        store.save(&config).expect("save config");

        assert_eq!(store.load().expect("reload config"), config);
    }

    #[test]
    fn upsert_entry_replaces_existing_local_path() {
        let repo = temp_repo_root("sync-config-upsert");
        let store = FileRepoSyncConfigStore::new(&repo);

        store
            .upsert_entry(sample_entry("private", SyncEntryKind::Directory))
            .expect("insert entry");

        let mut replacement = sample_entry("private", SyncEntryKind::File);
        replacement.server_profile_id = "workbox".to_owned();
        replacement.remote_relative_path = PathBuf::from("remote/private.txt");

        store
            .upsert_entry(replacement.clone())
            .expect("replace entry");

        let loaded = store.load().expect("load config");
        assert_eq!(loaded.entries, vec![replacement]);
    }

    #[test]
    fn remove_entry_deletes_only_matching_path() {
        let repo = temp_repo_root("sync-config-remove");
        let store = FileRepoSyncConfigStore::new(&repo);
        store
            .save(&RepoSyncConfig {
                entries: vec![
                    sample_entry("private", SyncEntryKind::Directory),
                    sample_entry("notes/today.html", SyncEntryKind::File),
                ],
                history_compaction_policies: vec![],
            })
            .expect("save config");

        assert!(
            store
                .remove_entry(Path::new("private"))
                .expect("remove entry")
        );
        assert!(
            !store
                .remove_entry(Path::new("does-not-exist"))
                .expect("remove missing entry")
        );

        let loaded = store.load().expect("load config");
        assert_eq!(
            loaded.entries,
            vec![sample_entry("notes/today.html", SyncEntryKind::File)]
        );
    }

    #[test]
    fn history_compaction_policies_round_trip_and_inherit() {
        let repo = temp_repo_root("sync-config-compact-policy");
        let store = FileRepoSyncConfigStore::new(&repo);
        store
            .upsert_history_compaction_policy(
                Path::new("private"),
                HistoryCompactionPolicy {
                    revisions: 12,
                    frequency: 4,
                },
            )
            .expect("save ancestor override");
        store
            .upsert_history_compaction_policy(
                Path::new("private/notes/today.html"),
                HistoryCompactionPolicy {
                    revisions: 3,
                    frequency: 2,
                },
            )
            .expect("save file override");

        let loaded = store.load().expect("load config");
        assert_eq!(loaded.history_compaction_policies.len(), 2);

        assert_eq!(
            store
                .resolve_history_compaction_policy(Path::new("notes/tomorrow.html"))
                .expect("resolve default"),
            ResolvedHistoryCompactionPolicy {
                policy: HistoryCompactionPolicy {
                    revisions: 100,
                    frequency: 10,
                },
                source: HistoryCompactionPolicySource::Default,
            }
        );

        assert_eq!(
            store
                .resolve_history_compaction_policy(Path::new("private/archive/plan.html"))
                .expect("resolve inherited policy"),
            ResolvedHistoryCompactionPolicy {
                policy: HistoryCompactionPolicy {
                    revisions: 12,
                    frequency: 4,
                },
                source: HistoryCompactionPolicySource::AncestorOverride {
                    repo_relative_path: PathBuf::from("private"),
                },
            }
        );

        assert_eq!(
            store
                .resolve_history_compaction_policy(Path::new("private/notes/today.html"))
                .expect("resolve direct override"),
            ResolvedHistoryCompactionPolicy {
                policy: HistoryCompactionPolicy {
                    revisions: 3,
                    frequency: 2,
                },
                source: HistoryCompactionPolicySource::PathOverride {
                    repo_relative_path: PathBuf::from("private/notes/today.html"),
                },
            }
        );
    }

    #[test]
    fn remove_history_compaction_policy_deletes_only_matching_path() {
        let repo = temp_repo_root("sync-config-compact-clear");
        let store = FileRepoSyncConfigStore::new(&repo);
        store
            .upsert_history_compaction_policy(
                Path::new("private"),
                HistoryCompactionPolicy {
                    revisions: 12,
                    frequency: 4,
                },
            )
            .expect("save ancestor override");
        store
            .upsert_history_compaction_policy(
                Path::new("private/notes/today.html"),
                HistoryCompactionPolicy {
                    revisions: 3,
                    frequency: 2,
                },
            )
            .expect("save file override");

        assert!(
            store
                .remove_history_compaction_policy(Path::new("private/notes/today.html"))
                .expect("remove file override")
        );
        assert!(
            !store
                .remove_history_compaction_policy(Path::new("does-not-exist"))
                .expect("remove missing override")
        );

        assert_eq!(
            store
                .resolve_history_compaction_policy(Path::new("private/notes/today.html"))
                .expect("resolve inherited after clear"),
            ResolvedHistoryCompactionPolicy {
                policy: HistoryCompactionPolicy {
                    revisions: 12,
                    frequency: 4,
                },
                source: HistoryCompactionPolicySource::AncestorOverride {
                    repo_relative_path: PathBuf::from("private"),
                },
            }
        );
    }
}
