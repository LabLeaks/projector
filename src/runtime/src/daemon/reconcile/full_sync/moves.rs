/**
@module PROJECTOR.RUNTIME.FULL_SYNC_MOVES
Detects conservative full-sync document moves by comparing previously materialized known documents with newly discovered local text files that match by body.
*/
// @fileimplements PROJECTOR.RUNTIME.FULL_SYNC_MOVES
use std::collections::{BTreeSet, HashMap};
use std::error::Error;
use std::fs;
use std::io;
use std::path::PathBuf;

use projector_domain::{BootstrapSnapshot, DocumentId, ManifestEntry, SyncContext};

use super::discovery::discover_local_text_files;

#[derive(Clone, Debug, Eq, PartialEq)]
struct CreatedTextCandidate {
    mount_relative_path: PathBuf,
    relative_path: PathBuf,
    text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct MoveOperation {
    pub(super) document_id: DocumentId,
    pub(super) mount_relative_path: PathBuf,
    pub(super) relative_path: PathBuf,
}

pub(super) fn detect_full_sync_moves(
    binding: &dyn SyncContext,
    current_snapshot: &BootstrapSnapshot,
    previously_materialized_paths: &BTreeSet<(PathBuf, PathBuf)>,
) -> Result<Vec<MoveOperation>, Box<dyn Error>> {
    let mounts_by_relative_path = binding
        .projection_mounts()
        .into_iter()
        .map(|mount| (mount.relative_path.clone(), mount))
        .collect::<HashMap<_, _>>();
    let removed_entries = current_snapshot
        .manifest
        .entries
        .iter()
        .filter(|entry| !entry.deleted)
        .filter(|entry| {
            previously_materialized_paths.contains(&(
                entry.mount_relative_path.clone(),
                entry.relative_path.clone(),
            ))
        })
        .filter(|entry| {
            mounts_by_relative_path
                .get(&entry.mount_relative_path)
                .map(|mount| {
                    let absolute_path = match mount.kind {
                        projector_domain::SyncEntryKind::Directory => {
                            mount.absolute_path.join(&entry.relative_path)
                        }
                        projector_domain::SyncEntryKind::File => mount.absolute_path.clone(),
                    };
                    !absolute_path.exists()
                })
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();

    let known_paths = current_snapshot
        .manifest
        .entries
        .iter()
        .filter(|entry| !entry.deleted)
        .map(|entry| {
            (
                entry.mount_relative_path.clone(),
                entry.relative_path.clone(),
            )
        })
        .collect::<BTreeSet<_>>();
    let mut created_candidates = Vec::new();
    for mount in binding.projection_mounts() {
        for relative_path in discover_local_text_files(&mount.absolute_path, &mount.kind)? {
            if known_paths.contains(&(mount.relative_path.clone(), relative_path.clone())) {
                continue;
            }
            let absolute_path = match mount.kind {
                projector_domain::SyncEntryKind::Directory => {
                    mount.absolute_path.join(&relative_path)
                }
                projector_domain::SyncEntryKind::File => mount.absolute_path.clone(),
            };
            let text = match fs::read_to_string(&absolute_path) {
                Ok(text) => text,
                Err(err) if err.kind() == io::ErrorKind::InvalidData => continue,
                Err(err) => return Err(err.into()),
            };
            created_candidates.push(CreatedTextCandidate {
                mount_relative_path: mount.relative_path.clone(),
                relative_path,
                text,
            });
        }
    }

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
            mount_relative_path: candidate.mount_relative_path.clone(),
            relative_path: candidate.relative_path.clone(),
        });
    }

    Ok(moves)
}
