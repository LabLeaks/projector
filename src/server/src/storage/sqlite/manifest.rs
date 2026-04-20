/**
@module PROJECTOR.SERVER.SQLITE_MANIFEST
Owns SQLite manifest and body mutation transactions for document create, update, delete, and move operations.
*/
// @fileimplements PROJECTOR.SERVER.SQLITE_MANIFEST
use std::path::PathBuf;

use projector_domain::{
    CreateDocumentRequest, DeleteDocumentRequest, DocumentId, DocumentKind, ManifestEntry,
    MoveDocumentRequest, ProvenanceEventKind, UpdateDocumentRequest,
};

use super::super::StoreError;
use super::super::body_persistence::{SnapshotBodyPersistence, SqliteBodyPersistence};
use super::super::body_state::{
    BodyConvergenceEngine, BodyStateModel, FULL_TEXT_BODY_MODEL, YrsConvergenceBodyEngine,
};
use super::state::{
    append_event, append_path_revision, display_document_path, load_required_workspace_state,
    make_document_id, make_event, save_workspace_state,
};

pub(super) fn create_document_tx(
    transaction: &rusqlite::Transaction<'_>,
    request: &CreateDocumentRequest,
) -> Result<DocumentId, StoreError> {
    let mut state = load_required_workspace_state(transaction, &request.workspace_id)?;
    enforce_manifest_cursor(state.cursor, request.based_on_cursor)?;

    let requested_mount = PathBuf::from(&request.mount_relative_path);
    if !state.metadata.mounts.contains(&requested_mount) {
        return Err(StoreError::new(format!(
            "workspace {} is not bound to mount {}",
            request.workspace_id, request.mount_relative_path
        )));
    }

    if state.snapshot.manifest.entries.iter().any(|entry| {
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
    state.snapshot.manifest.entries.push(ManifestEntry {
        document_id: document_id.clone(),
        mount_relative_path: requested_mount,
        relative_path: PathBuf::from(&request.relative_path),
        kind: DocumentKind::Text,
        deleted: false,
    });
    let body_persistence = SqliteBodyPersistence::new(transaction, &request.workspace_id);
    let initial_state = FULL_TEXT_BODY_MODEL.state_from_materialized_text(request.text.clone());
    body_persistence.write_current_state(&mut state.snapshot, &document_id, &initial_state)?;

    let event = make_event(
        &mut state,
        &request.actor_id,
        Some(document_id.clone()),
        Some(request.mount_relative_path.clone()),
        Some(request.relative_path.clone()),
        format!(
            "created text document at {}",
            display_document_path(&request.mount_relative_path, &request.relative_path)
        ),
        ProvenanceEventKind::DocumentCreated,
    );
    save_workspace_state(transaction, &state)?;
    append_event(transaction, &request.workspace_id, &event)?;
    body_persistence.append_retained_history(
        event.cursor,
        &request.actor_id,
        document_id.as_str(),
        &FULL_TEXT_BODY_MODEL.created_history(&initial_state),
        event.timestamp_ms,
    )?;
    append_path_revision(
        transaction,
        &request.workspace_id,
        &super::super::history::FilePathRevision {
            seq: event.cursor,
            workspace_cursor: event.cursor,
            actor_id: request.actor_id.clone(),
            document_id: document_id.as_str().to_owned(),
            mount_path: request.mount_relative_path.clone(),
            relative_path: request.relative_path.clone(),
            deleted: false,
            event_kind: "document_created".to_owned(),
            timestamp_ms: event.timestamp_ms,
        },
    )?;
    Ok(document_id)
}

pub(super) fn update_document_tx(
    transaction: &rusqlite::Transaction<'_>,
    request: &UpdateDocumentRequest,
) -> Result<(), StoreError> {
    let mut state = load_required_workspace_state(transaction, &request.workspace_id)?;
    let document_id = DocumentId::new(&request.document_id);
    let Some(entry) = state
        .snapshot
        .manifest
        .entries
        .iter()
        .find(|entry| !entry.deleted && entry.document_id == document_id)
        .cloned()
    else {
        return Err(StoreError::new(format!(
            "document {} is not live in workspace {}",
            request.document_id, request.workspace_id
        )));
    };

    let body_persistence = SqliteBodyPersistence::new(transaction, &request.workspace_id);
    let current_state = body_persistence.load_current_state(&state.snapshot, &document_id)?;
    let merge = YrsConvergenceBodyEngine.apply_update(
        &request.actor_id,
        &request.base_text,
        &current_state,
        &request.text,
    );
    body_persistence.write_current_state(
        &mut state.snapshot,
        &document_id,
        merge.canonical_state(),
    )?;

    let event = make_event(
        &mut state,
        &request.actor_id,
        Some(document_id.clone()),
        Some(entry.mount_relative_path.display().to_string()),
        Some(entry.relative_path.display().to_string()),
        merge.summary_for_path(&entry.mount_relative_path, &entry.relative_path),
        ProvenanceEventKind::DocumentUpdated,
    );
    save_workspace_state(transaction, &state)?;
    append_event(transaction, &request.workspace_id, &event)?;
    body_persistence.append_retained_history(
        event.cursor,
        &request.actor_id,
        &request.document_id,
        merge.retained_history(),
        event.timestamp_ms,
    )
}

pub(super) fn delete_document_tx(
    transaction: &rusqlite::Transaction<'_>,
    request: &DeleteDocumentRequest,
) -> Result<(), StoreError> {
    let mut state = load_required_workspace_state(transaction, &request.workspace_id)?;
    enforce_manifest_cursor(state.cursor, request.based_on_cursor)?;

    let document_id = DocumentId::new(&request.document_id);
    let Some(entry) = state
        .snapshot
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
    let mount_path = entry.mount_relative_path.display().to_string();
    let relative_path = entry.relative_path.display().to_string();
    entry.deleted = true;
    state
        .snapshot
        .bodies
        .retain(|body| body.document_id != document_id);

    let event = make_event(
        &mut state,
        &request.actor_id,
        Some(document_id.clone()),
        Some(mount_path.clone()),
        Some(relative_path.clone()),
        format!(
            "deleted text document at {}",
            display_document_path(&mount_path, &relative_path)
        ),
        ProvenanceEventKind::DocumentDeleted,
    );
    save_workspace_state(transaction, &state)?;
    append_event(transaction, &request.workspace_id, &event)?;
    append_path_revision(
        transaction,
        &request.workspace_id,
        &super::super::history::FilePathRevision {
            seq: event.cursor,
            workspace_cursor: event.cursor,
            actor_id: request.actor_id.clone(),
            document_id: request.document_id.clone(),
            mount_path,
            relative_path,
            deleted: true,
            event_kind: "document_deleted".to_owned(),
            timestamp_ms: event.timestamp_ms,
        },
    )
}

pub(super) fn move_document_tx(
    transaction: &rusqlite::Transaction<'_>,
    request: &MoveDocumentRequest,
) -> Result<(), StoreError> {
    let mut state = load_required_workspace_state(transaction, &request.workspace_id)?;
    enforce_manifest_cursor(state.cursor, request.based_on_cursor)?;

    let requested_mount = PathBuf::from(&request.mount_relative_path);
    if !state.metadata.mounts.contains(&requested_mount) {
        return Err(StoreError::new(format!(
            "workspace {} is not bound to mount {}",
            request.workspace_id, request.mount_relative_path
        )));
    }

    if state.snapshot.manifest.entries.iter().any(|entry| {
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
        state.snapshot.manifest.entries.iter_mut().find(|entry| {
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

    let event = make_event(
        &mut state,
        &request.actor_id,
        Some(DocumentId::new(&request.document_id)),
        Some(request.mount_relative_path.clone()),
        Some(request.relative_path.clone()),
        format!(
            "moved text document to {}",
            display_document_path(&request.mount_relative_path, &request.relative_path)
        ),
        ProvenanceEventKind::DocumentMoved,
    );
    save_workspace_state(transaction, &state)?;
    append_event(transaction, &request.workspace_id, &event)?;
    append_path_revision(
        transaction,
        &request.workspace_id,
        &super::super::history::FilePathRevision {
            seq: event.cursor,
            workspace_cursor: event.cursor,
            actor_id: request.actor_id.clone(),
            document_id: request.document_id.clone(),
            mount_path: request.mount_relative_path.clone(),
            relative_path: request.relative_path.clone(),
            deleted: false,
            event_kind: "document_moved".to_owned(),
            timestamp_ms: event.timestamp_ms,
        },
    )
}

pub(super) fn enforce_manifest_cursor(
    current: u64,
    based_on_cursor: Option<u64>,
) -> Result<(), StoreError> {
    let provided = based_on_cursor
        .ok_or_else(|| StoreError::new("manifest write missing based_on_cursor precondition"))?;
    if provided == current {
        return Ok(());
    }
    Err(StoreError::conflict(
        "stale_cursor",
        format!(
            "manifest write based on stale cursor {provided}; current workspace cursor is {current}"
        ),
    ))
}
