/**
@module PROJECTOR.RUNTIME.SYNC_TARGETS
Loads runnable sync-entry targets directly from repo-local sync-entry configuration so the machine daemon can operate on one synced path at a time.
*/
// @fileimplements PROJECTOR.RUNTIME.SYNC_TARGETS
use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use projector_domain::{ProjectionMount, RepoSyncConfig, SyncEntryKind, SyncEntryTarget};

use crate::{FileRepoSyncConfigStore, FileServerProfileStore, ProjectorHome};

pub fn load_sync_targets(
    repo_root: &Path,
    home: &ProjectorHome,
) -> Result<Vec<SyncEntryTarget>, io::Error> {
    let sync_config_store = FileRepoSyncConfigStore::new(repo_root);
    let mut sync_config = sync_config_store.load()?;
    if relocate_missing_sync_entry_roots(repo_root, &mut sync_config)? {
        sync_config_store.save(&sync_config)?;
    }
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

fn relocate_missing_sync_entry_roots(
    repo_root: &Path,
    config: &mut RepoSyncConfig,
) -> Result<bool, io::Error> {
    let materialized_paths = load_materialized_paths(repo_root)?;
    if materialized_paths.is_empty() {
        return Ok(false);
    }

    let mut changed = false;
    let mut occupied_existing_roots = config
        .entries
        .iter()
        .filter(|entry| repo_root.join(&entry.local_relative_path).exists())
        .map(|entry| entry.local_relative_path.clone())
        .collect::<BTreeSet<_>>();

    for entry in &mut config.entries {
        if entry.kind != SyncEntryKind::Directory {
            continue;
        }
        if repo_root.join(&entry.local_relative_path).exists() {
            continue;
        }

        let expected_paths = materialized_paths
            .iter()
            .filter(|materialized| materialized.mount_relative_path == entry.remote_relative_path)
            .filter(|materialized| !materialized.relative_path.as_os_str().is_empty())
            .filter(|materialized| materialized.text_fingerprint.is_some())
            .cloned()
            .collect::<Vec<_>>();
        if expected_paths.is_empty() {
            continue;
        }

        let candidates =
            find_relocated_directory_roots(repo_root, &expected_paths, &occupied_existing_roots)?;
        if candidates.len() != 1 {
            continue;
        }

        entry.local_relative_path = candidates[0].clone();
        occupied_existing_roots.insert(candidates[0].clone());
        changed = true;
    }

    if changed {
        config
            .entries
            .sort_by(|left, right| left.local_relative_path.cmp(&right.local_relative_path));
    }
    Ok(changed)
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct MaterializedPath {
    mount_relative_path: PathBuf,
    relative_path: PathBuf,
    text_fingerprint: Option<String>,
}

fn load_materialized_paths(repo_root: &Path) -> Result<BTreeSet<MaterializedPath>, io::Error> {
    let path = repo_root.join(".projector/materialized_paths.txt");
    if !path.exists() {
        return Ok(BTreeSet::new());
    }

    let mut paths = BTreeSet::new();
    for line in fs::read_to_string(path)?.lines() {
        let parts = line.split('\t').collect::<Vec<_>>();
        let (mount, relative, text_fingerprint) = match parts.as_slice() {
            [mount, relative] => (mount, relative, None),
            [mount, relative, fingerprint] => (mount, relative, Some((*fingerprint).to_owned())),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid materialized path line: expected 2 or 3 tab-separated fields",
                ));
            }
        };
        paths.insert(MaterializedPath {
            mount_relative_path: PathBuf::from(mount),
            relative_path: PathBuf::from(relative),
            text_fingerprint,
        });
    }
    Ok(paths)
}

fn find_relocated_directory_roots(
    repo_root: &Path,
    expected_paths: &[MaterializedPath],
    occupied_existing_roots: &BTreeSet<PathBuf>,
) -> Result<Vec<PathBuf>, io::Error> {
    let mut candidates = Vec::new();
    collect_relocated_directory_roots(
        repo_root,
        repo_root,
        expected_paths,
        occupied_existing_roots,
        &mut candidates,
    )?;
    candidates.sort();
    Ok(candidates)
}

fn collect_relocated_directory_roots(
    repo_root: &Path,
    current: &Path,
    expected_paths: &[MaterializedPath],
    occupied_existing_roots: &BTreeSet<PathBuf>,
    candidates: &mut Vec<PathBuf>,
) -> Result<(), io::Error> {
    if is_repo_metadata_or_build_dir(repo_root, current) {
        return Ok(());
    }

    let relative_current = current
        .strip_prefix(repo_root)
        .map_err(|err| io::Error::other(err.to_string()))?;
    if is_within_occupied_existing_root(relative_current, occupied_existing_roots) {
        return Ok(());
    }
    if !relative_current.as_os_str().is_empty()
        && is_gitignored(repo_root, relative_current)
        && relocated_directory_matches_expected_paths(current, expected_paths)
    {
        candidates.push(relative_current.to_path_buf());
    }

    let entries = match fs::read_dir(current) {
        Ok(entries) => entries,
        Err(_) => return Ok(()),
    };
    for entry in entries {
        let Ok(entry) = entry else {
            continue;
        };
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            collect_relocated_directory_roots(
                repo_root,
                &entry.path(),
                expected_paths,
                occupied_existing_roots,
                candidates,
            )?;
        }
    }

    Ok(())
}

fn relocated_directory_matches_expected_paths(
    current: &Path,
    expected_paths: &[MaterializedPath],
) -> bool {
    expected_paths.iter().all(|expected_path| {
        let path = current.join(&expected_path.relative_path);
        if !path.is_file() {
            return false;
        }
        let Some(expected_fingerprint) = expected_path.text_fingerprint.as_deref() else {
            return false;
        };
        fs::read_to_string(path)
            .map(|text| text_fingerprint(&text) == expected_fingerprint)
            .unwrap_or(false)
    })
}

fn is_gitignored(repo_root: &Path, relative_path: &Path) -> bool {
    Command::new("git")
        .args(["check-ignore", "-q", "--"])
        .arg(relative_path)
        .current_dir(repo_root)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn text_fingerprint(text: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn is_within_occupied_existing_root(
    relative_path: &Path,
    occupied_existing_roots: &BTreeSet<PathBuf>,
) -> bool {
    !relative_path.as_os_str().is_empty()
        && occupied_existing_roots
            .iter()
            .any(|root| relative_path.starts_with(root))
}

fn is_repo_metadata_or_build_dir(repo_root: &Path, current: &Path) -> bool {
    if current == repo_root {
        return false;
    }
    let Ok(relative_path) = current.strip_prefix(repo_root) else {
        return true;
    };
    matches!(
        relative_path.components().next().and_then(|component| {
            let value = component.as_os_str().to_string_lossy();
            matches!(value.as_ref(), ".git" | ".jj" | ".projector" | "target").then_some(())
        }),
        Some(())
    )
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io;
    use std::path::PathBuf;
    use std::process::Command;

    use projector_domain::{ActorId, RepoSyncConfig, RepoSyncEntry, SyncEntryKind, WorkspaceId};

    use crate::{FileRepoSyncConfigStore, ProjectorHome, test_support::temp_repo_root};

    use super::{derive_sync_targets, load_sync_targets, text_fingerprint};

    fn init_git_repo(repo_root: &std::path::Path) {
        let status = Command::new("git")
            .arg("init")
            .arg("-q")
            .current_dir(repo_root)
            .status()
            .expect("git init");
        assert!(status.success(), "git init failed");
    }

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
        };

        let targets = derive_sync_targets(&repo_root, &config, None).expect("derive targets");
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].mount.relative_path, PathBuf::from("notes"));
        assert_eq!(targets[0].server_addr.as_deref(), Some("homebox"));
        assert_eq!(targets[1].mount.relative_path, PathBuf::from("private"));
        assert_eq!(targets[1].server_addr.as_deref(), Some("homebox"));
    }

    // @verifies PROJECTOR.SYNC.ROOT_RENAME_PRESERVES_SYNC_ENTRY_BINDINGS
    #[test]
    fn load_sync_targets_relocates_missing_directory_sync_entry_root() {
        let repo_root = temp_repo_root("sync-target-root-rename");
        let projector_home = repo_root.join("projector-home");
        let config_store = FileRepoSyncConfigStore::new(&repo_root);
        init_git_repo(&repo_root);
        fs::create_dir_all(repo_root.join(".projector")).expect("create projector dir");
        fs::write(repo_root.join(".gitignore"), "sourcerodeo/private/\n").expect("write gitignore");
        fs::write(
            repo_root.join(".projector/materialized_paths.txt"),
            format!(
                "private\tbriefs/index.md\t{}\nprivate\treference.md\t{}\n",
                text_fingerprint("brief\n"),
                text_fingerprint("reference\n")
            ),
        )
        .expect("write materialized paths");
        fs::create_dir_all(repo_root.join("sourcerodeo/private/briefs"))
            .expect("create relocated root");
        fs::write(
            repo_root.join("sourcerodeo/private/briefs/index.md"),
            "brief\n",
        )
        .expect("write relocated child");
        fs::write(
            repo_root.join("sourcerodeo/private/reference.md"),
            "reference\n",
        )
        .expect("write relocated root child");
        config_store
            .save(&RepoSyncConfig {
                entries: vec![RepoSyncEntry {
                    entry_id: "entry-private".to_owned(),
                    workspace_id: WorkspaceId::new("ws-private"),
                    actor_id: ActorId::new("actor-private"),
                    server_profile_id: "homebox".to_owned(),
                    local_relative_path: PathBuf::from("private"),
                    remote_relative_path: PathBuf::from("private"),
                    kind: SyncEntryKind::Directory,
                }],
            })
            .expect("save sync config");

        let targets = load_sync_targets(&repo_root, &ProjectorHome::new(&projector_home))
            .expect("load relocated sync targets");
        let config = config_store.load().expect("reload sync config");

        assert_eq!(targets.len(), 1);
        assert_eq!(
            targets[0].mount.absolute_path,
            repo_root.join("sourcerodeo/private")
        );
        assert_eq!(
            config.entries[0].local_relative_path,
            PathBuf::from("sourcerodeo/private")
        );
        assert_eq!(
            config.entries[0].remote_relative_path,
            PathBuf::from("private")
        );
    }

    // @verifies PROJECTOR.SYNC.ROOT_RENAME_PRESERVES_SYNC_ENTRY_BINDINGS
    #[test]
    fn load_sync_targets_does_not_relocate_into_existing_sync_entry_root() {
        let repo_root = temp_repo_root("sync-target-root-overlap");
        let projector_home = repo_root.join("projector-home");
        let config_store = FileRepoSyncConfigStore::new(&repo_root);
        init_git_repo(&repo_root);
        fs::create_dir_all(repo_root.join(".projector")).expect("create projector dir");
        fs::write(repo_root.join(".gitignore"), "notes/\n").expect("write gitignore");
        fs::write(
            repo_root.join(".projector/materialized_paths.txt"),
            format!(
                "private\tarchive/secret.md\t{}\n",
                text_fingerprint("secret\n")
            ),
        )
        .expect("write materialized paths");
        fs::create_dir_all(repo_root.join("notes/archive")).expect("create existing root child");
        fs::write(repo_root.join("notes/archive/secret.md"), "secret\n")
            .expect("write matching child under occupied root");
        config_store
            .save(&RepoSyncConfig {
                entries: vec![
                    RepoSyncEntry {
                        entry_id: "entry-private".to_owned(),
                        workspace_id: WorkspaceId::new("ws-private"),
                        actor_id: ActorId::new("actor-private"),
                        server_profile_id: "homebox".to_owned(),
                        local_relative_path: PathBuf::from("private"),
                        remote_relative_path: PathBuf::from("private"),
                        kind: SyncEntryKind::Directory,
                    },
                    RepoSyncEntry {
                        entry_id: "entry-notes".to_owned(),
                        workspace_id: WorkspaceId::new("ws-notes"),
                        actor_id: ActorId::new("actor-notes"),
                        server_profile_id: "homebox".to_owned(),
                        local_relative_path: PathBuf::from("notes"),
                        remote_relative_path: PathBuf::from("notes"),
                        kind: SyncEntryKind::Directory,
                    },
                ],
            })
            .expect("save sync config");

        let _targets = load_sync_targets(&repo_root, &ProjectorHome::new(&projector_home))
            .expect("load sync targets");
        let config = config_store.load().expect("reload sync config");

        assert_eq!(
            config.entries[0].local_relative_path,
            PathBuf::from("private")
        );
        assert_eq!(
            config.entries[1].local_relative_path,
            PathBuf::from("notes")
        );
    }

    // @verifies PROJECTOR.SYNC.ROOT_RENAME_PRESERVES_SYNC_ENTRY_BINDINGS
    #[test]
    fn load_sync_targets_does_not_relocate_to_unignored_matching_directory() {
        let repo_root = temp_repo_root("sync-target-root-unignored-match");
        let projector_home = repo_root.join("projector-home");
        let config_store = FileRepoSyncConfigStore::new(&repo_root);
        init_git_repo(&repo_root);
        fs::create_dir_all(repo_root.join(".projector")).expect("create projector dir");
        fs::write(
            repo_root.join(".projector/materialized_paths.txt"),
            format!(
                "private\tREADME.md\t{}\n",
                text_fingerprint("private readme\n")
            ),
        )
        .expect("write materialized paths");
        fs::create_dir_all(repo_root.join("docs")).expect("create unrelated docs");
        fs::write(repo_root.join("docs/README.md"), "private readme\n")
            .expect("write matching unignored file");
        config_store
            .save(&RepoSyncConfig {
                entries: vec![RepoSyncEntry {
                    entry_id: "entry-private".to_owned(),
                    workspace_id: WorkspaceId::new("ws-private"),
                    actor_id: ActorId::new("actor-private"),
                    server_profile_id: "homebox".to_owned(),
                    local_relative_path: PathBuf::from("private"),
                    remote_relative_path: PathBuf::from("private"),
                    kind: SyncEntryKind::Directory,
                }],
            })
            .expect("save sync config");

        let _targets = load_sync_targets(&repo_root, &ProjectorHome::new(&projector_home))
            .expect("load sync targets");
        let config = config_store.load().expect("reload sync config");

        assert_eq!(
            config.entries[0].local_relative_path,
            PathBuf::from("private")
        );
    }

    // @verifies PROJECTOR.SYNC.ROOT_RENAME_PRESERVES_SYNC_ENTRY_BINDINGS
    #[test]
    fn load_sync_targets_does_not_relocate_when_content_fingerprint_differs() {
        let repo_root = temp_repo_root("sync-target-root-content-mismatch");
        let projector_home = repo_root.join("projector-home");
        let config_store = FileRepoSyncConfigStore::new(&repo_root);
        init_git_repo(&repo_root);
        fs::create_dir_all(repo_root.join(".projector")).expect("create projector dir");
        fs::write(repo_root.join(".gitignore"), "candidate/\n").expect("write gitignore");
        fs::write(
            repo_root.join(".projector/materialized_paths.txt"),
            format!(
                "private\tREADME.md\t{}\n",
                text_fingerprint("private readme\n")
            ),
        )
        .expect("write materialized paths");
        fs::create_dir_all(repo_root.join("candidate")).expect("create candidate");
        fs::write(repo_root.join("candidate/README.md"), "public readme\n")
            .expect("write mismatched file");
        config_store
            .save(&RepoSyncConfig {
                entries: vec![RepoSyncEntry {
                    entry_id: "entry-private".to_owned(),
                    workspace_id: WorkspaceId::new("ws-private"),
                    actor_id: ActorId::new("actor-private"),
                    server_profile_id: "homebox".to_owned(),
                    local_relative_path: PathBuf::from("private"),
                    remote_relative_path: PathBuf::from("private"),
                    kind: SyncEntryKind::Directory,
                }],
            })
            .expect("save sync config");

        let _targets = load_sync_targets(&repo_root, &ProjectorHome::new(&projector_home))
            .expect("load sync targets");
        let config = config_store.load().expect("reload sync config");

        assert_eq!(
            config.entries[0].local_relative_path,
            PathBuf::from("private")
        );
    }

    #[test]
    fn load_sync_targets_rejects_malformed_materialized_path_records() {
        let repo_root = temp_repo_root("sync-target-invalid-materialized-path");
        fs::create_dir_all(repo_root.join(".projector")).expect("create projector dir");
        fs::write(
            repo_root.join(".projector/materialized_paths.txt"),
            "private\tREADME.md\tfingerprint\textra\n",
        )
        .expect("write invalid materialized paths");

        let err = super::load_materialized_paths(&repo_root)
            .expect_err("load materialized paths rejects extra field");

        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }
}
