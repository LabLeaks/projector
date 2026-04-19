/**
@module PROJECTOR.RUNTIME.SYNC_TARGETS
Loads runnable sync-entry targets directly from repo-local sync-entry configuration so the machine daemon can operate on one synced path at a time.
*/
// @fileimplements PROJECTOR.RUNTIME.SYNC_TARGETS
use std::io;
use std::path::Path;

use projector_domain::{ProjectionMount, RepoSyncConfig, SyncEntryTarget};

use crate::{FileRepoSyncConfigStore, FileServerProfileStore, ProjectorHome};

pub fn load_sync_targets(
    repo_root: &Path,
    home: &ProjectorHome,
) -> Result<Vec<SyncEntryTarget>, io::Error> {
    let sync_config = FileRepoSyncConfigStore::new(repo_root).load()?;
    let server_profiles = FileServerProfileStore::new(home.clone());
    derive_sync_targets(repo_root, &sync_config, Some(&server_profiles))
}

pub fn derive_sync_targets(
    repo_root: &Path,
    config: &RepoSyncConfig,
    server_profiles: Option<&FileServerProfileStore>,
) -> Result<Vec<SyncEntryTarget>, io::Error> {
    let projector_dir = repo_root.join(".projector");
    let source_repo_name = repo_root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned());
    let mut targets = config
        .entries
        .iter()
        .map(|entry| -> Result<SyncEntryTarget, io::Error> {
            Ok(SyncEntryTarget {
                entry_id: entry.entry_id.clone(),
                workspace_id: entry.workspace_id.clone(),
                actor_id: entry.actor_id.clone(),
                server_addr: match server_profiles {
                    Some(server_profiles) => server_profiles
                        .resolve_profile(&entry.server_profile_id)?
                        .map(|profile| profile.server_addr)
                        .or_else(|| Some(entry.server_profile_id.clone())),
                    None => Some(entry.server_profile_id.clone()),
                },
                projector_dir: projector_dir.clone(),
                source_repo_name: source_repo_name.clone(),
                mount: ProjectionMount {
                    relative_path: entry.remote_relative_path.clone(),
                    absolute_path: repo_root.join(&entry.local_relative_path),
                    kind: entry.kind.clone(),
                },
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    targets.sort_by(|left, right| {
        left.mount
            .relative_path
            .cmp(&right.mount.relative_path)
            .then(left.actor_id.as_str().cmp(right.actor_id.as_str()))
            .then(left.server_addr.cmp(&right.server_addr))
    });
    Ok(targets)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use projector_domain::{ActorId, RepoSyncConfig, RepoSyncEntry, SyncEntryKind, WorkspaceId};

    use super::derive_sync_targets;

    #[test]
    fn derive_sync_targets_creates_one_target_per_sync_entry() {
        let repo_root = std::env::temp_dir().join("projector-runtime-sync-targets");
        let config = RepoSyncConfig {
            entries: vec![
                RepoSyncEntry {
                    entry_id: "entry-private".to_owned(),
                    workspace_id: WorkspaceId::new("ws-1"),
                    actor_id: ActorId::new("actor-a"),
                    server_profile_id: "homebox".to_owned(),
                    local_relative_path: PathBuf::from("private"),
                    remote_relative_path: PathBuf::from("private"),
                    kind: SyncEntryKind::Directory,
                },
                RepoSyncEntry {
                    entry_id: "entry-notes".to_owned(),
                    workspace_id: WorkspaceId::new("ws-1"),
                    actor_id: ActorId::new("actor-a"),
                    server_profile_id: "homebox".to_owned(),
                    local_relative_path: PathBuf::from("notes"),
                    remote_relative_path: PathBuf::from("notes"),
                    kind: SyncEntryKind::Directory,
                },
            ],
            history_compaction_policies: vec![],
        };

        let targets = derive_sync_targets(&repo_root, &config, None).expect("derive targets");
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].mount.relative_path, PathBuf::from("notes"));
        assert_eq!(targets[0].server_addr.as_deref(), Some("homebox"));
        assert_eq!(targets[1].mount.relative_path, PathBuf::from("private"));
        assert_eq!(targets[1].server_addr.as_deref(), Some("homebox"));
    }
}
