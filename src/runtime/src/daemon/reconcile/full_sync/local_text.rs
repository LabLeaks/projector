/**
@module PROJECTOR.RUNTIME.FULL_SYNC_LOCAL_TEXT
Applies full-sync local text create, update, and delete operations against the server using discovered UTF-8 text files across configured projection mounts.
*/
// @fileimplements PROJECTOR.RUNTIME.FULL_SYNC_LOCAL_TEXT
use std::collections::{BTreeSet, HashMap};
use std::error::Error;
use std::fs;
use std::io;
use std::path::PathBuf;

use projector_domain::{BootstrapSnapshot, ProjectionMount, SyncContext};

use super::discovery::discover_local_text_files;
use super::materialized_bodies::load_materialized_body_texts;
use crate::Transport;

pub(super) fn push_local_only_text_documents(
    binding: &dyn SyncContext,
    snapshot: &BootstrapSnapshot,
    current_cursor: u64,
    transport: &mut impl Transport<Error = io::Error>,
) -> Result<(Vec<(PathBuf, PathBuf)>, u64), Box<dyn Error>> {
    let known_paths = snapshot
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

    let mut created = Vec::new();
    let mut manifest_cursor = current_cursor;
    for mount in binding.projection_mounts() {
        for relative_path in discover_local_text_files(&mount.absolute_path, &mount.kind)? {
            if known_paths.contains(&(mount.relative_path.clone(), relative_path.clone())) {
                continue;
            }
            let local_path = match mount.kind {
                projector_domain::SyncEntryKind::Directory => {
                    mount.absolute_path.join(&relative_path)
                }
                projector_domain::SyncEntryKind::File => mount.absolute_path.clone(),
            };
            let text = match fs::read_to_string(local_path) {
                Ok(text) => text,
                Err(err) if err.kind() == io::ErrorKind::InvalidData => continue,
                Err(err) => return Err(err.into()),
            };
            transport.create_document(
                binding,
                manifest_cursor,
                &mount.relative_path,
                &relative_path,
                &text,
            )?;
            manifest_cursor += 1;
            created.push((mount.relative_path.clone(), relative_path));
        }
    }

    created.sort();
    Ok((created, manifest_cursor))
}

pub(super) fn push_local_text_updates(
    binding: &dyn SyncContext,
    snapshot: &BootstrapSnapshot,
    transport: &mut impl Transport<Error = io::Error>,
) -> Result<Vec<(PathBuf, PathBuf)>, Box<dyn Error>> {
    let materialized_body_texts =
        load_materialized_body_texts(binding.projector_dir()).unwrap_or_default();
    let mounts_by_relative_path = binding
        .projection_mounts()
        .into_iter()
        .map(|mount| (mount.relative_path.clone(), mount))
        .collect::<HashMap<_, _>>();
    let body_by_id = snapshot
        .bodies
        .iter()
        .map(|body| (body.document_id.clone(), body.text.as_str()))
        .collect::<HashMap<_, _>>();

    let mut updated = Vec::new();
    for entry in snapshot
        .manifest
        .entries
        .iter()
        .filter(|entry| !entry.deleted)
    {
        let absolute_path = absolute_path_for_entry(
            mounts_by_relative_path.get(&entry.mount_relative_path),
            entry,
        )?;
        let local_text = match fs::read_to_string(&absolute_path) {
            Ok(text) => text,
            Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
            Err(err) if err.kind() == io::ErrorKind::InvalidData => continue,
            Err(err) => return Err(err.into()),
        };
        let remote_text = body_by_id
            .get(&entry.document_id)
            .ok_or_else(|| format!("snapshot missing body for {}", entry.document_id.as_str()))?;
        if local_text == *remote_text {
            continue;
        }
        let base_text = materialized_body_texts
            .get(&entry.document_id)
            .map(String::as_str)
            .unwrap_or(remote_text);
        transport.update_document(binding, &entry.document_id, base_text, &local_text)?;
        updated.push((
            entry.mount_relative_path.clone(),
            entry.relative_path.clone(),
        ));
    }

    updated.sort();
    Ok(updated)
}

pub(super) fn push_local_text_deletions(
    binding: &dyn SyncContext,
    snapshot: &BootstrapSnapshot,
    previously_materialized_paths: &BTreeSet<(PathBuf, PathBuf)>,
    current_cursor: u64,
    transport: &mut impl Transport<Error = io::Error>,
) -> Result<(Vec<(PathBuf, PathBuf)>, u64), Box<dyn Error>> {
    let mut deleted = Vec::new();
    let mut manifest_cursor = current_cursor;
    for entry in snapshot
        .manifest
        .entries
        .iter()
        .filter(|entry| !entry.deleted)
    {
        let mounts_by_relative_path = binding
            .projection_mounts()
            .into_iter()
            .map(|mount| (mount.relative_path.clone(), mount))
            .collect::<HashMap<_, _>>();
        let absolute_path = absolute_path_for_entry(
            mounts_by_relative_path.get(&entry.mount_relative_path),
            entry,
        )?;
        if !previously_materialized_paths.contains(&(
            entry.mount_relative_path.clone(),
            entry.relative_path.clone(),
        )) {
            continue;
        }
        if absolute_path.exists() {
            continue;
        }
        transport.delete_document(binding, manifest_cursor, &entry.document_id)?;
        manifest_cursor += 1;
        deleted.push((
            entry.mount_relative_path.clone(),
            entry.relative_path.clone(),
        ));
    }

    deleted.sort();
    Ok((deleted, manifest_cursor))
}

fn absolute_path_for_entry(
    mount: Option<&ProjectionMount>,
    entry: &projector_domain::ManifestEntry,
) -> Result<PathBuf, Box<dyn Error>> {
    let Some(mount) = mount else {
        return Err(format!(
            "snapshot references unknown mount {}",
            entry.mount_relative_path.display()
        )
        .into());
    };
    Ok(match mount.kind {
        projector_domain::SyncEntryKind::Directory => {
            mount.absolute_path.join(&entry.relative_path)
        }
        projector_domain::SyncEntryKind::File => mount.absolute_path.clone(),
    })
}
