/**
@module PROJECTOR.TESTS.SUPPORT.LOCAL_BOOTSTRAP_BINDING_SUPPORT
Binding, profile, and repo-sync-config convenience helpers for local-bootstrap proofs.
*/
// @fileimplements PROJECTOR.TESTS.SUPPORT.LOCAL_BOOTSTRAP_BINDING_SUPPORT
use super::*;

pub(crate) fn connect_profile(
    repo_root: &Path,
    projector_home: &Path,
    profile_id: &str,
    addr: &str,
) {
    run_projector_home(
        repo_root,
        projector_home,
        &["connect", "--id", profile_id, "--server", addr],
    );
}

pub(crate) fn add_sync_entry(repo_root: &Path, projector_home: &Path, addr: &str, path: &str) {
    connect_profile(repo_root, projector_home, "homebox", addr);
    run_projector_home(repo_root, projector_home, &["add", path]);
}

pub(crate) fn load_workspace_binding_from_sync_config(repo_root: &Path) -> CheckoutBinding {
    let config = FileRepoSyncConfigStore::new(repo_root)
        .load()
        .expect("load sync config");
    let targets = derive_sync_targets(repo_root, &config, None).expect("derive sync targets");
    let first = targets.first().expect("configured sync targets");
    CheckoutBinding {
        workspace_id: first.workspace_id.clone(),
        actor_id: first.actor_id.clone(),
        server_addr: first.server_addr.clone(),
        roots: ProjectionRoots {
            projector_dir: repo_root.join(".projector"),
            projection_paths: targets
                .iter()
                .map(|target| target.mount.absolute_path.clone())
                .collect(),
        },
        projection_relative_paths: targets
            .iter()
            .map(|target| target.mount.relative_path.clone())
            .collect(),
        projection_kinds: targets
            .iter()
            .map(|target| target.mount.kind.clone())
            .collect(),
    }
}

pub(crate) fn clone_sync_config_for_repo(source_repo: &Path, dest_repo: &Path, actor_id: &str) {
    let config = FileRepoSyncConfigStore::new(source_repo)
        .load()
        .expect("load source sync config");
    let cloned = RepoSyncConfig {
        entries: config
            .entries
            .into_iter()
            .map(|mut entry| {
                entry.actor_id = ActorId::new(actor_id);
                entry
            })
            .collect(),
    };
    FileRepoSyncConfigStore::new(dest_repo)
        .save(&cloned)
        .expect("save cloned sync config");
}

pub(crate) fn save_sync_config_for_binding(repo_root: &Path, binding: &CheckoutBinding) {
    let config = RepoSyncConfig {
        entries: binding
            .projection_relative_paths
            .iter()
            .cloned()
            .zip(binding.projection_kinds.iter().cloned())
            .map(|(path, kind)| RepoSyncEntry {
                entry_id: format!("entry-{}", path.display()),
                workspace_id: binding.workspace_id.clone(),
                actor_id: binding.actor_id.clone(),
                server_profile_id: binding
                    .server_addr
                    .clone()
                    .unwrap_or_else(|| "none".to_owned()),
                local_relative_path: path.clone(),
                remote_relative_path: path,
                kind,
            })
            .collect(),
    };
    FileRepoSyncConfigStore::new(repo_root)
        .save(&config)
        .expect("save sync config for binding");
}
