/**
@module PROJECTOR.RUNTIME.WATCH_MOVES
Detects conservative watcher-driven document move candidates by resolving touched projection paths and matching removed known documents to newly created files with identical bodies.
*/
// @fileimplements PROJECTOR.RUNTIME.WATCH_MOVES
use std::collections::{BTreeSet, HashMap};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use projector_domain::{BootstrapSnapshot, DocumentId, ManifestEntry, SyncContext};

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
    events: &[WatcherEvent],
) -> BTreeSet<TouchedProjectionPath> {
    let mut touched = BTreeSet::new();
    for event in events {
        let absolute_path = match event {
            WatcherEvent::FileChanged(path)
            | WatcherEvent::FileCreated(path)
            | WatcherEvent::FileDeleted(path) => path,
        };
        if let Some(path) = resolve_projection_path(binding, absolute_path) {
            touched.insert(path);
        }
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

fn resolve_projection_path(
    binding: &dyn SyncContext,
    absolute_path: &Path,
) -> Option<TouchedProjectionPath> {
    for mount in binding.projection_mounts() {
        match mount.kind {
            projector_domain::SyncEntryKind::Directory => {
                if let Ok(relative_path) = absolute_path.strip_prefix(&mount.absolute_path) {
                    if relative_path.as_os_str().is_empty() {
                        return None;
                    }
                    if absolute_path.exists() && absolute_path.is_dir() {
                        return None;
                    }
                    return Some(TouchedProjectionPath {
                        mount_relative_path: mount.relative_path.clone(),
                        relative_path: relative_path.to_path_buf(),
                        absolute_path: absolute_path.to_path_buf(),
                    });
                }
            }
            projector_domain::SyncEntryKind::File => {
                if absolute_path == mount.absolute_path {
                    return Some(TouchedProjectionPath {
                        mount_relative_path: mount.relative_path.clone(),
                        relative_path: PathBuf::new(),
                        absolute_path: absolute_path.to_path_buf(),
                    });
                }
            }
        }
    }
    None
}
