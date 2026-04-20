/**
@module PROJECTOR.SERVER.FILE_MANIFEST
Owns file-backed document path lifecycle rules, mount validation, and stale-cursor checks for create, move, and delete operations.
*/
// @fileimplements PROJECTOR.SERVER.FILE_MANIFEST
use std::path::{Path, PathBuf};

use projector_domain::{
    CreateDocumentRequest, DeleteDocumentRequest, DocumentId, DocumentKind, ManifestEntry,
    MoveDocumentRequest, ProvenanceEvent, ProvenanceEventKind,
};

use super::StoreError;
use super::bodies::{file_persist_workspace_snapshot, file_read_workspace_snapshot};
use super::body_persistence::{FileBodyPersistence, SnapshotBodyPersistence};
use super::body_state::{BodyStateModel, FULL_TEXT_BODY_MODEL};
use super::history::{FilePathRevision, file_append_path_revision};
use super::manifest::{
    display_document_path, file_enforce_manifest_cursor, make_document_id, now_ms,
};
use super::provenance::{file_append_workspace_event, file_workspace_cursor};
use super::workspaces::file_read_bound_mounts;

pub(crate) fn file_create_document(
    state_dir: &Path,
    request: &CreateDocumentRequest,
) -> Result<DocumentId, StoreError> {
    file_enforce_manifest_cursor(state_dir, &request.workspace_id, request.based_on_cursor)?;

    let existing_mounts = file_read_bound_mounts(state_dir, &request.workspace_id)?;
    let requested_mount = PathBuf::from(&request.mount_relative_path);
    if !existing_mounts.contains(&requested_mount) {
        return Err(StoreError::new(format!(
            "workspace {} is not bound to mount {}",
            request.workspace_id, request.mount_relative_path
        )));
    }

    let mut snapshot = file_read_workspace_snapshot(state_dir, &request.workspace_id)?;
    if snapshot.manifest.entries.iter().any(|entry| {
        !entry.deleted
            && entry.mount_relative_path == requested_mount
            && entry.relative_path == PathBuf::from(&request.relative_path)
    }) {
        return Err(StoreError::conflict(
            "path_taken",
            format!(
                "document already exists at {}/{}",
                request.mount_relative_path, request.relative_path
            ),
        ));
    }

    let document_id = DocumentId::new(make_document_id());
    snapshot.manifest.entries.push(ManifestEntry {
        document_id: document_id.clone(),
        mount_relative_path: requested_mount,
        relative_path: PathBuf::from(&request.relative_path),
        kind: DocumentKind::Text,
        deleted: false,
    });
    let body_persistence = FileBodyPersistence::new(state_dir, &request.workspace_id);
    let initial_state = FULL_TEXT_BODY_MODEL.state_from_materialized_text(request.text.clone());
    body_persistence.write_current_state(&mut snapshot, &document_id, &initial_state);
    file_persist_workspace_snapshot(state_dir, &request.workspace_id, &snapshot)?;
    let event_cursor = file_workspace_cursor(state_dir, &request.workspace_id)? + 1;
    file_append_workspace_event(
        state_dir,
        &request.workspace_id,
        ProvenanceEvent {
            cursor: event_cursor,
            timestamp_ms: now_ms(),
            actor_id: projector_domain::ActorId::new(request.actor_id.clone()),
            document_id: Some(document_id.clone()),
            mount_relative_path: Some(request.mount_relative_path.clone()),
            relative_path: Some(request.relative_path.clone()),
            summary: format!(
                "created text document at {}",
                display_document_path(&request.mount_relative_path, &request.relative_path)
            ),
            kind: ProvenanceEventKind::DocumentCreated,
        },
    )?;
    body_persistence.append_retained_history(
        event_cursor,
        &request.actor_id,
        document_id.as_str(),
        &FULL_TEXT_BODY_MODEL.created_history(&initial_state),
        now_ms(),
    )?;
    file_append_path_revision(
        state_dir,
        &request.workspace_id,
        FilePathRevision {
            seq: event_cursor,
            workspace_cursor: event_cursor,
            actor_id: request.actor_id.clone(),
            document_id: document_id.as_str().to_owned(),
            mount_path: request.mount_relative_path.clone(),
            relative_path: request.relative_path.clone(),
            deleted: false,
            event_kind: "document_created".to_owned(),
            timestamp_ms: now_ms(),
        },
    )?;
    Ok(document_id)
}

pub(crate) fn file_delete_document(
    state_dir: &Path,
    request: &DeleteDocumentRequest,
) -> Result<(), StoreError> {
    file_enforce_manifest_cursor(state_dir, &request.workspace_id, request.based_on_cursor)?;

    let mut snapshot = file_read_workspace_snapshot(state_dir, &request.workspace_id)?;
    let document_id = DocumentId::new(&request.document_id);
    let Some(entry) = snapshot
        .manifest
        .entries
        .iter_mut()
        .find(|entry| !entry.deleted && entry.document_id == document_id)
    else {
        return Err(StoreError::new(format!(
            "document {} is not live in workspace {}",
            request.document_id, request.workspace_id
        )));
    };
    let mount_relative_path = entry.mount_relative_path.display().to_string();
    let relative_path = entry.relative_path.display().to_string();
    entry.deleted = true;
    snapshot
        .bodies
        .retain(|body| body.document_id != document_id);
    file_persist_workspace_snapshot(state_dir, &request.workspace_id, &snapshot)?;
    let event_cursor = file_workspace_cursor(state_dir, &request.workspace_id)? + 1;
    file_append_workspace_event(
        state_dir,
        &request.workspace_id,
        ProvenanceEvent {
            cursor: event_cursor,
            timestamp_ms: now_ms(),
            actor_id: projector_domain::ActorId::new(request.actor_id.clone()),
            document_id: Some(document_id),
            mount_relative_path: Some(mount_relative_path.clone()),
            relative_path: Some(relative_path.clone()),
            summary: format!(
                "deleted text document at {}",
                display_document_path(&mount_relative_path, &relative_path)
            ),
            kind: ProvenanceEventKind::DocumentDeleted,
        },
    )?;
    file_append_path_revision(
        state_dir,
        &request.workspace_id,
        FilePathRevision {
            seq: event_cursor,
            workspace_cursor: event_cursor,
            actor_id: request.actor_id.clone(),
            document_id: request.document_id.clone(),
            mount_path: mount_relative_path,
            relative_path,
            deleted: true,
            event_kind: "document_deleted".to_owned(),
            timestamp_ms: now_ms(),
        },
    )?;
    Ok(())
}

pub(crate) fn file_move_document(
    state_dir: &Path,
    request: &MoveDocumentRequest,
) -> Result<(), StoreError> {
    file_enforce_manifest_cursor(state_dir, &request.workspace_id, request.based_on_cursor)?;

    let existing_mounts = file_read_bound_mounts(state_dir, &request.workspace_id)?;
    let requested_mount = PathBuf::from(&request.mount_relative_path);
    if !existing_mounts.contains(&requested_mount) {
        return Err(StoreError::new(format!(
            "workspace {} is not bound to mount {}",
            request.workspace_id, request.mount_relative_path
        )));
    }

    let mut snapshot = file_read_workspace_snapshot(state_dir, &request.workspace_id)?;
    if snapshot.manifest.entries.iter().any(|entry| {
        entry.document_id != DocumentId::new(&request.document_id)
            && !entry.deleted
            && entry.mount_relative_path == requested_mount
            && entry.relative_path == PathBuf::from(&request.relative_path)
    }) {
        return Err(StoreError::conflict(
            "path_taken",
            format!(
                "document already exists at {}/{}",
                request.mount_relative_path, request.relative_path
            ),
        ));
    }

    let Some(entry) =
        snapshot.manifest.entries.iter_mut().find(|entry| {
            !entry.deleted && entry.document_id == DocumentId::new(&request.document_id)
        })
    else {
        return Err(StoreError::new(format!(
            "document {} is not live in workspace {}",
            request.document_id, request.workspace_id
        )));
    };

    entry.mount_relative_path = requested_mount;
    entry.relative_path = PathBuf::from(&request.relative_path);
    file_persist_workspace_snapshot(state_dir, &request.workspace_id, &snapshot)?;
    let event_cursor = file_workspace_cursor(state_dir, &request.workspace_id)? + 1;
    file_append_workspace_event(
        state_dir,
        &request.workspace_id,
        ProvenanceEvent {
            cursor: event_cursor,
            timestamp_ms: now_ms(),
            actor_id: projector_domain::ActorId::new(request.actor_id.clone()),
            document_id: Some(DocumentId::new(&request.document_id)),
            mount_relative_path: Some(request.mount_relative_path.clone()),
            relative_path: Some(request.relative_path.clone()),
            summary: format!(
                "moved text document to {}",
                display_document_path(&request.mount_relative_path, &request.relative_path)
            ),
            kind: ProvenanceEventKind::DocumentMoved,
        },
    )?;
    file_append_path_revision(
        state_dir,
        &request.workspace_id,
        FilePathRevision {
            seq: event_cursor,
            workspace_cursor: event_cursor,
            actor_id: request.actor_id.clone(),
            document_id: request.document_id.clone(),
            mount_path: request.mount_relative_path.clone(),
            relative_path: request.relative_path.clone(),
            deleted: false,
            event_kind: "document_moved".to_owned(),
            timestamp_ms: now_ms(),
        },
    )?;
    Ok(())
}
