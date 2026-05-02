/**
@module PROJECTOR.RUNTIME.MACHINE_DAEMON_REPOS
Owns repo-runtime refresh and watch-root derivation for the machine-global daemon.
*/
// @fileimplements PROJECTOR.RUNTIME.MACHINE_DAEMON_REPOS
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::PathBuf;

use projector_domain::SyncEntryTarget;

use crate::{FileMachineSyncRegistryStore, ProjectorHome, WatchedMount, load_sync_targets};

use super::RepoRuntime;

pub(super) fn refresh_repo_runtimes(
    repo_registry_store: &FileMachineSyncRegistryStore,
    home: &ProjectorHome,
    runtimes: &mut BTreeMap<PathBuf, RepoRuntime>,
) -> Result<(), io::Error> {
    let registry = repo_registry_store.load()?;
    let wanted_roots = registry
        .repos
        .iter()
        .map(|repo| repo.repo_root.clone())
        .collect::<BTreeSet<_>>();
    runtimes.retain(|repo_root, _| wanted_roots.contains(repo_root));

    for registered_repo in registry.repos {
        let sync_targets = match load_sync_targets(&registered_repo.repo_root, home) {
            Ok(sync_targets) if !sync_targets.is_empty() => sync_targets,
            Ok(_) => {
                runtimes.remove(&registered_repo.repo_root);
                continue;
            }
            Err(_) => continue,
        };

        let should_replace = runtimes
            .get(&registered_repo.repo_root)
            .map(|runtime| runtime.sync_targets != sync_targets)
            .unwrap_or(true);
        if should_replace {
            let runtime = RepoRuntime::new(sync_targets)?;
            runtimes.insert(registered_repo.repo_root, runtime);
        }
    }

    Ok(())
}

pub(super) fn watched_mounts(sync_targets: &[SyncEntryTarget]) -> Vec<WatchedMount> {
    let mut mounts = sync_targets
        .iter()
        .map(|target| WatchedMount {
            absolute_path: target.mount.absolute_path.clone(),
            kind: target.mount.kind.clone(),
        })
        .collect::<Vec<_>>();
    mounts.sort_by(|left, right| left.absolute_path.cmp(&right.absolute_path));
    mounts.dedup_by(|left, right| {
        left.absolute_path == right.absolute_path && left.kind == right.kind
    });
    mounts
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use crate::global_state::{MachineSyncRegistry, RegisteredRepo};
    use crate::machine_daemon::RepoRuntime;
    use crate::test_support::temp_projector_home;
    use crate::{FileMachineSyncRegistryStore, ProjectorHome};

    use super::refresh_repo_runtimes;

    #[test]
    fn refresh_repo_runtimes_keeps_missing_repos_registered() {
        let home_path = temp_projector_home("machine-daemon-missing-repo");
        let home = ProjectorHome::new(&home_path);
        let registry_store = FileMachineSyncRegistryStore::new(home.clone());
        let missing_repo = home_path.join("missing-repo");
        registry_store
            .save(&MachineSyncRegistry {
                repos: vec![RegisteredRepo {
                    repo_root: missing_repo.clone(),
                    repo_identity: Some("repo-missing".to_owned()),
                    entry_count: 1,
                    server_profiles: vec!["homebox".to_owned()],
                    updated_at_ms: 1,
                }],
            })
            .expect("save registry");
        let mut runtimes = BTreeMap::<PathBuf, RepoRuntime>::new();

        refresh_repo_runtimes(&registry_store, &home, &mut runtimes).expect("refresh runtimes");

        assert!(runtimes.is_empty());
        assert_eq!(
            registry_store.load().expect("load registry").repos[0].repo_root,
            missing_repo
        );
    }
}
