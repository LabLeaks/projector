/**
@module PROJECTOR.RUNTIME.WATCH_MOVES
Detects conservative watcher-driven document move candidates by resolving touched projection paths and matching removed known documents to newly created files with identical bodies.
*/
// @fileimplements PROJECTOR.RUNTIME.WATCH_MOVES
use std::collections::{BTreeSet, HashMap};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use projector_domain::{BootstrapSnapshot, DocumentId, ManifestEntry, SyncContext, SyncEntryKind};

use crate::WatcherEvent;

#[derive(Clone, Debug, Eq, PartialEq)]
struct CreatedTextCandidate {
    path: TouchedProjectionPath,
    text: String,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct TouchedProjectionPath {
    pub(super) mount_relative_path: PathBuf,
    pub(super) relative_path: PathBuf,
    pub(super) absolute_path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct MoveOperation {
    pub(super) document_id: DocumentId,
    pub(super) mount_relative_path: PathBuf,
    pub(super) relative_path: PathBuf,
}

pub(super) fn touched_projection_paths(
    binding: &dyn SyncContext,
    current_snapshot: &BootstrapSnapshot,
    events: &[WatcherEvent],
) -> BTreeSet<TouchedProjectionPath> {
    let mut touched = BTreeSet::new();
    for event in events {
        let absolute_path = match event {
            WatcherEvent::FileChanged(path)
            | WatcherEvent::FileCreated(path)
            | WatcherEvent::FileDeleted(path) => path,
        };
        expand_projection_path(binding, absolute_path, current_snapshot, &mut touched);
    }
    touched
}

pub(super) fn detect_touched_path_moves(
    current_snapshot: &BootstrapSnapshot,
    touched_paths: &BTreeSet<TouchedProjectionPath>,
) -> Result<Vec<MoveOperation>, Box<dyn Error>> {
    let live_entries = current_snapshot
        .manifest
        .entries
        .iter()
        .filter(|entry| !entry.deleted)
        .collect::<Vec<_>>();
    let removed_entries = live_entries
        .into_iter()
        .filter(|entry| {
            touched_paths.iter().any(|path| {
                path.mount_relative_path == entry.mount_relative_path
                    && path.relative_path == entry.relative_path
                    && !path.absolute_path.exists()
            })
        })
        .collect::<Vec<_>>();
    let created_candidates = touched_paths
        .iter()
        .filter(|path| path.absolute_path.exists())
        .filter(|path| path.absolute_path.is_file())
        .filter(|path| {
            !current_snapshot.manifest.entries.iter().any(|entry| {
                !entry.deleted
                    && entry.mount_relative_path == path.mount_relative_path
                    && entry.relative_path == path.relative_path
            })
        })
        .filter_map(|path| {
            fs::read_to_string(&path.absolute_path)
                .ok()
                .map(|text| CreatedTextCandidate {
                    path: path.clone(),
                    text,
                })
        })
        .collect::<Vec<_>>();

    match_move_candidates(current_snapshot, removed_entries, &created_candidates)
}

fn match_move_candidates(
    current_snapshot: &BootstrapSnapshot,
    removed_entries: Vec<&ManifestEntry>,
    created_candidates: &[CreatedTextCandidate],
) -> Result<Vec<MoveOperation>, Box<dyn Error>> {
    let body_by_id = current_snapshot
        .bodies
        .iter()
        .map(|body| (body.document_id.clone(), body.text.as_str()))
        .collect::<HashMap<_, _>>();
    let mut used_candidate_indexes = BTreeSet::new();
    let mut moves = Vec::new();

    for entry in removed_entries {
        let Some(remote_text) = body_by_id.get(&entry.document_id) else {
            continue;
        };
        let matching = created_candidates
            .iter()
            .enumerate()
            .filter(|(index, candidate)| {
                !used_candidate_indexes.contains(index) && candidate.text == *remote_text
            })
            .collect::<Vec<_>>();
        if matching.len() != 1 {
            continue;
        }

        let (candidate_index, candidate) = matching[0];
        used_candidate_indexes.insert(candidate_index);
        moves.push(MoveOperation {
            document_id: entry.document_id.clone(),
            mount_relative_path: candidate.path.mount_relative_path.clone(),
            relative_path: candidate.path.relative_path.clone(),
        });
    }

    Ok(moves)
}

fn expand_projection_path(
    binding: &dyn SyncContext,
    absolute_path: &Path,
    current_snapshot: &BootstrapSnapshot,
    touched: &mut BTreeSet<TouchedProjectionPath>,
) {
    for mount in binding.projection_mounts() {
        match mount.kind {
            SyncEntryKind::Directory => {
                if !mount.absolute_path.exists() && absolute_path.starts_with(&mount.absolute_path)
                {
                    return;
                }
                if let Ok(relative_path) = absolute_path.strip_prefix(&mount.absolute_path) {
                    if absolute_path.exists() && absolute_path.is_dir() {
                        append_existing_directory_children(
                            &mount.relative_path,
                            &mount.absolute_path,
                            relative_path,
                            absolute_path,
                            touched,
                        );
                        return;
                    }

                    touched.insert(TouchedProjectionPath {
                        mount_relative_path: mount.relative_path.clone(),
                        relative_path: relative_path.to_path_buf(),
                        absolute_path: absolute_path.to_path_buf(),
                    });
                    if !absolute_path.exists() {
                        append_known_children_under_missing_path(
                            current_snapshot,
                            &mount.relative_path,
                            relative_path,
                            &mount.absolute_path,
                            touched,
                        );
                    }
                    return;
                }
            }
            SyncEntryKind::File => {
                if absolute_path == mount.absolute_path {
                    touched.insert(TouchedProjectionPath {
                        mount_relative_path: mount.relative_path.clone(),
                        relative_path: PathBuf::new(),
                        absolute_path: absolute_path.to_path_buf(),
                    });
                    return;
                }
            }
        }
    }
}

fn append_existing_directory_children(
    mount_relative_path: &Path,
    mount_absolute_path: &Path,
    directory_relative_path: &Path,
    directory_absolute_path: &Path,
    touched: &mut BTreeSet<TouchedProjectionPath>,
) {
    if let Ok(entries) = fs::read_dir(directory_absolute_path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Ok(child_relative_path) = path.strip_prefix(mount_absolute_path) {
                    append_existing_directory_children(
                        mount_relative_path,
                        mount_absolute_path,
                        child_relative_path,
                        &path,
                        touched,
                    );
                }
                continue;
            }
            if !path.is_file() {
                continue;
            }
            if let Ok(child_relative_path) = path.strip_prefix(mount_absolute_path) {
                if child_relative_path.starts_with(directory_relative_path) {
                    touched.insert(TouchedProjectionPath {
                        mount_relative_path: mount_relative_path.to_path_buf(),
                        relative_path: child_relative_path.to_path_buf(),
                        absolute_path: path,
                    });
                }
            }
        }
    }
}

fn append_known_children_under_missing_path(
    current_snapshot: &BootstrapSnapshot,
    mount_relative_path: &Path,
    missing_relative_path: &Path,
    mount_absolute_path: &Path,
    touched: &mut BTreeSet<TouchedProjectionPath>,
) {
    for entry in &current_snapshot.manifest.entries {
        if entry.deleted || entry.mount_relative_path != mount_relative_path {
            continue;
        }
        if !entry.relative_path.starts_with(missing_relative_path) {
            continue;
        }
        touched.insert(TouchedProjectionPath {
            mount_relative_path: mount_relative_path.to_path_buf(),
            relative_path: entry.relative_path.clone(),
            absolute_path: mount_absolute_path.join(&entry.relative_path),
        });
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use projector_domain::{
        ActorId, BootstrapSnapshot, DocumentBody, DocumentId, DocumentKind, ManifestEntry,
        ManifestState, ProjectionMount, SyncContext, SyncEntryKind, WorkspaceId,
    };

    use super::{detect_touched_path_moves, touched_projection_paths};
    use crate::WatcherEvent;

    struct TestSyncContext {
        projector_dir: PathBuf,
        workspace_id: WorkspaceId,
        actor_id: ActorId,
        mounts: Vec<ProjectionMount>,
    }

    impl SyncContext for TestSyncContext {
        fn workspace_id(&self) -> &WorkspaceId {
            &self.workspace_id
        }

        fn actor_id(&self) -> &ActorId {
            &self.actor_id
        }

        fn projector_dir(&self) -> &Path {
            &self.projector_dir
        }

        fn projection_mounts(&self) -> Vec<ProjectionMount> {
            self.mounts.clone()
        }

        fn server_addr(&self) -> Option<&str> {
            None
        }

        fn source_repo_name(&self) -> Option<&str> {
            None
        }
    }

    // @verifies PROJECTOR.SYNC.FOLDER_RENAME_PRESERVES_DOCUMENTS
    #[test]
    fn directory_rename_events_expand_to_child_file_move_candidates() {
        let root = temp_dir("watch-folder-rename");
        let mount = root.join("private");
        fs::create_dir_all(mount.join("archive")).expect("create archive directory");
        fs::write(mount.join("archive/index.md"), "move me\n").expect("write moved file");

        let binding = TestSyncContext {
            projector_dir: root.join(".projector"),
            workspace_id: WorkspaceId::new("ws-test"),
            actor_id: ActorId::new("actor-test"),
            mounts: vec![ProjectionMount {
                relative_path: PathBuf::from("private"),
                absolute_path: mount.clone(),
                kind: SyncEntryKind::Directory,
            }],
        };
        let snapshot = BootstrapSnapshot {
            manifest: ManifestState {
                entries: vec![ManifestEntry {
                    document_id: DocumentId::new("doc-rename"),
                    mount_relative_path: PathBuf::from("private"),
                    relative_path: PathBuf::from("briefs/index.md"),
                    kind: DocumentKind::Text,
                    deleted: false,
                }],
            },
            bodies: vec![DocumentBody {
                document_id: DocumentId::new("doc-rename"),
                text: "move me\n".to_owned(),
            }],
        };

        let touched = touched_projection_paths(
            &binding,
            &snapshot,
            &[
                WatcherEvent::FileDeleted(mount.join("briefs")),
                WatcherEvent::FileCreated(mount.join("archive")),
            ],
        );
        let moves = detect_touched_path_moves(&snapshot, &touched).expect("detect moves");

        assert_eq!(moves.len(), 1);
        assert_eq!(moves[0].document_id, DocumentId::new("doc-rename"));
        assert_eq!(moves[0].mount_relative_path, Path::new("private"));
        assert_eq!(moves[0].relative_path, Path::new("archive/index.md"));
    }

    #[test]
    fn missing_mount_root_suppresses_child_delete_events() {
        let root = temp_dir("watch-missing-root");
        let mount = root.join("private");

        let binding = TestSyncContext {
            projector_dir: root.join(".projector"),
            workspace_id: WorkspaceId::new("ws-test"),
            actor_id: ActorId::new("actor-test"),
            mounts: vec![ProjectionMount {
                relative_path: PathBuf::from("private"),
                absolute_path: mount.clone(),
                kind: SyncEntryKind::Directory,
            }],
        };
        let snapshot = BootstrapSnapshot {
            manifest: ManifestState {
                entries: vec![ManifestEntry {
                    document_id: DocumentId::new("doc-missing-root"),
                    mount_relative_path: PathBuf::from("private"),
                    relative_path: PathBuf::from("briefs/index.md"),
                    kind: DocumentKind::Text,
                    deleted: false,
                }],
            },
            bodies: vec![DocumentBody {
                document_id: DocumentId::new("doc-missing-root"),
                text: "keep me\n".to_owned(),
            }],
        };

        let touched = touched_projection_paths(
            &binding,
            &snapshot,
            &[WatcherEvent::FileDeleted(mount.join("briefs/index.md"))],
        );

        assert!(touched.is_empty());
    }

    fn temp_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("projector-{name}-{unique}"));
        fs::create_dir_all(&path).expect("create temp dir");
        path
    }
}
