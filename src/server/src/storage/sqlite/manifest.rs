/**
@module PROJECTOR.SERVER.SQLITE_MANIFEST
Owns SQLite manifest and body mutation transactions for document create, update, delete, and move operations.
*/
// @fileimplements PROJECTOR.SERVER.SQLITE_MANIFEST
use std::path::PathBuf;

use projector_domain::{
    CreateDocumentRequest, DeleteDocumentRequest, DocumentBody, DocumentId, DocumentKind,
    ManifestEntry, MoveDocumentRequest, ProvenanceEventKind, UpdateDocumentRequest,
};

use super::super::StoreError;
use super::super::bodies::merge_text_update;
use super::state::{
    append_body_revision, append_event, append_path_revision, display_document_path,
    load_required_workspace_state, make_document_id, make_event, save_workspace_state, upsert_body,
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
    state.snapshot.bodies.push(DocumentBody {
        document_id: document_id.clone(),
        text: request.text.clone(),
    });

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
    append_body_revision(
        transaction,
        &request.workspace_id,
        &super::super::history::FileBodyRevision {
            seq: event.cursor,
            workspace_cursor: event.cursor,
            actor_id: request.actor_id.clone(),
            document_id: document_id.as_str().to_owned(),
            base_text: String::new(),
            body_text: request.text.clone(),
            conflicted: false,
            timestamp_ms: event.timestamp_ms,
        },
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

    let current_text = state
        .snapshot
        .bodies
        .iter()
        .find(|body| body.document_id == document_id)
        .map(|body| body.text.clone())
        .unwrap_or_default();
    let merge = merge_text_update(&request.base_text, &current_text, &request.text);
    upsert_body(&mut state.snapshot, &document_id, &merge.text);

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
    append_body_revision(
        transaction,
        &request.workspace_id,
        &super::super::history::FileBodyRevision {
            seq: event.cursor,
            workspace_cursor: event.cursor,
            actor_id: request.actor_id.clone(),
            document_id: request.document_id.clone(),
            base_text: request.base_text.clone(),
            body_text: merge.text,
            conflicted: merge.conflicted,
            timestamp_ms: event.timestamp_ms,
        },
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
