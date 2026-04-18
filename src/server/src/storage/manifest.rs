/**
@module PROJECTOR.SERVER.MANIFEST
Owns document path lifecycle rules, mount validation, and stale-cursor checks for create, move, and delete operations.
*/
// @fileimplements PROJECTOR.SERVER.MANIFEST
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use projector_domain::{
    CreateDocumentRequest, DeleteDocumentRequest, DocumentId, DocumentKind, ManifestEntry,
    MoveDocumentRequest, ProvenanceEvent, ProvenanceEventKind,
};

use super::body_state::{BodyStateModel, FULL_TEXT_BODY_MODEL};
use super::StoreError;
use super::bodies::{
    document_kind_db_value, file_persist_workspace_snapshot, file_read_workspace_snapshot,
};
use super::history::{
    FileBodyRevision, FilePathRevision, file_append_body_revision, file_append_path_revision,
    insert_body_revision_tx, insert_path_revision_tx,
};
use super::provenance::{
    current_workspace_cursor_tx, file_append_workspace_event, file_workspace_cursor,
    insert_event_tx,
};
use super::workspaces::file_read_bound_mounts;

fn display_document_path(mount_path: &str, relative_path: &str) -> String {
    if relative_path.is_empty() {
        mount_path.to_owned()
    } else {
        format!("{mount_path}/{relative_path}")
    }
}

pub(crate) fn file_enforce_manifest_cursor(
    state_dir: &Path,
    workspace_id: &str,
    based_on_cursor: Option<u64>,
) -> Result<(), StoreError> {
    let provided = based_on_cursor
        .ok_or_else(|| StoreError::new("manifest write missing based_on_cursor precondition"))?;
    let current = file_workspace_cursor(state_dir, workspace_id)?;
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
    snapshot.bodies.push(
        FULL_TEXT_BODY_MODEL
            .state_from_materialized_text(request.text.clone())
            .into_document_body(document_id.clone()),
    );
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
    file_append_body_revision(
        state_dir,
        &request.workspace_id,
        FileBodyRevision::from_retained_history(
            event_cursor,
            event_cursor,
            request.actor_id.clone(),
            document_id.as_str().to_owned(),
            &FULL_TEXT_BODY_MODEL
                .created_history(&FULL_TEXT_BODY_MODEL.state_from_materialized_text(
                    request.text.clone(),
                )),
            now_ms(),
        ),
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

pub(crate) async fn enforce_manifest_cursor_tx(
    transaction: &tokio_postgres::Transaction<'_>,
    workspace_id: &str,
    based_on_cursor: Option<u64>,
) -> Result<(), StoreError> {
    let provided = based_on_cursor
        .ok_or_else(|| StoreError::new("manifest write missing based_on_cursor precondition"))?;
    let current = current_workspace_cursor_tx(transaction, workspace_id).await?;
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

pub(crate) async fn postgres_create_document(
    transaction: &tokio_postgres::Transaction<'_>,
    request: &CreateDocumentRequest,
) -> Result<DocumentId, StoreError> {
    let document_id = DocumentId::new(make_document_id());

    enforce_manifest_cursor_tx(transaction, &request.workspace_id, request.based_on_cursor).await?;

    let workspace_exists = transaction
        .query_opt(
            "select id from workspaces where id = $1",
            &[&request.workspace_id],
        )
        .await?;
    if workspace_exists.is_none() {
        return Err(StoreError::new(format!(
            "workspace {} is not bound",
            request.workspace_id
        )));
    }

    let mount_exists = transaction
        .query_opt(
            "select 1 from workspace_mounts where workspace_id = $1 and mount_path = $2",
            &[&request.workspace_id, &request.mount_relative_path],
        )
        .await?;
    if mount_exists.is_none() {
        return Err(StoreError::new(format!(
            "workspace {} is not bound to mount {}",
            request.workspace_id, request.mount_relative_path
        )));
    }

    let existing_document = transaction
        .query_opt(
            "select document_id from document_paths \
             where workspace_id = $1 and mount_path = $2 and relative_path = $3 and deleted = false",
            &[
                &request.workspace_id,
                &request.mount_relative_path,
                &request.relative_path,
            ],
        )
        .await?;
    if existing_document.is_some() {
        return Err(StoreError::conflict(
            "path_taken",
            format!(
                "document already exists at {}/{}",
                request.mount_relative_path, request.relative_path
            ),
        ));
    }

    let kind = document_kind_db_value(&DocumentKind::Text);
    transaction
        .execute(
            "insert into documents (id, workspace_id, kind) values ($1, $2, $3)",
            &[&document_id.as_str(), &request.workspace_id, &kind],
        )
        .await?;
    transaction
        .execute(
            "insert into document_paths \
             (document_id, workspace_id, mount_path, relative_path, deleted, manifest_version) \
             values ($1, $2, $3, $4, false, 1)",
            &[
                &document_id.as_str(),
                &request.workspace_id,
                &request.mount_relative_path,
                &request.relative_path,
            ],
        )
        .await?;
    transaction
        .execute(
            "insert into document_body_snapshots \
             (document_id, workspace_id, body_text, compacted_through_seq) \
             values ($1, $2, $3, 0)",
            &[&document_id.as_str(), &request.workspace_id, &request.text],
        )
        .await?;
    let event_cursor = insert_event_tx(
        transaction,
        &request.workspace_id,
        &request.actor_id,
        Some(document_id.as_str()),
        Some(&request.mount_relative_path),
        Some(&request.relative_path),
        ProvenanceEventKind::DocumentCreated,
        &format!(
            "created text document at {}",
            display_document_path(&request.mount_relative_path, &request.relative_path)
        ),
    )
    .await?;
    insert_body_revision_tx(
        transaction,
        &request.workspace_id,
        document_id.as_str(),
        event_cursor,
        &request.actor_id,
        &FULL_TEXT_BODY_MODEL
            .created_history(&FULL_TEXT_BODY_MODEL.state_from_materialized_text(
                request.text.clone(),
            )),
    )
    .await?;
    insert_path_revision_tx(
        transaction,
        &request.workspace_id,
        document_id.as_str(),
        event_cursor,
        &request.actor_id,
        &request.mount_relative_path,
        &request.relative_path,
        false,
        "document_created",
    )
    .await?;

    Ok(document_id)
}

pub(crate) async fn postgres_delete_document(
    transaction: &tokio_postgres::Transaction<'_>,
    request: &DeleteDocumentRequest,
) -> Result<(), StoreError> {
    enforce_manifest_cursor_tx(transaction, &request.workspace_id, request.based_on_cursor).await?;

    let path_row = transaction
        .query_opt(
            "select mount_path, relative_path from document_paths \
             where workspace_id = $1 and document_id = $2 and deleted = false",
            &[&request.workspace_id, &request.document_id],
        )
        .await?;
    let Some(path_row) = path_row else {
        return Err(StoreError::new(format!(
            "document {} is not live in workspace {}",
            request.document_id, request.workspace_id
        )));
    };
    let mount_path = path_row.get::<_, String>("mount_path");
    let relative_path = path_row.get::<_, String>("relative_path");

    transaction
        .execute(
            "update document_paths set deleted = true, updated_at = now() \
             where workspace_id = $1 and document_id = $2",
            &[&request.workspace_id, &request.document_id],
        )
        .await?;
    let event_cursor = insert_event_tx(
        transaction,
        &request.workspace_id,
        &request.actor_id,
        Some(&request.document_id),
        Some(&mount_path),
        Some(&relative_path),
        ProvenanceEventKind::DocumentDeleted,
        &format!(
            "deleted text document at {}",
            display_document_path(&mount_path, &relative_path)
        ),
    )
    .await?;
    insert_path_revision_tx(
        transaction,
        &request.workspace_id,
        &request.document_id,
        event_cursor,
        &request.actor_id,
        &mount_path,
        &relative_path,
        true,
        "document_deleted",
    )
    .await?;

    Ok(())
}

pub(crate) async fn postgres_move_document(
    transaction: &tokio_postgres::Transaction<'_>,
    request: &MoveDocumentRequest,
) -> Result<(), StoreError> {
    enforce_manifest_cursor_tx(transaction, &request.workspace_id, request.based_on_cursor).await?;

    let mount_exists = transaction
        .query_opt(
            "select 1 from workspace_mounts where workspace_id = $1 and mount_path = $2",
            &[&request.workspace_id, &request.mount_relative_path],
        )
        .await?;
    if mount_exists.is_none() {
        return Err(StoreError::new(format!(
            "workspace {} is not bound to mount {}",
            request.workspace_id, request.mount_relative_path
        )));
    }

    let path_row = transaction
        .query_opt(
            "select document_id from document_paths \
             where workspace_id = $1 and document_id = $2 and deleted = false",
            &[&request.workspace_id, &request.document_id],
        )
        .await?;
    if path_row.is_none() {
        return Err(StoreError::new(format!(
            "document {} is not live in workspace {}",
            request.document_id, request.workspace_id
        )));
    }

    let existing_document = transaction
        .query_opt(
            "select document_id from document_paths \
             where workspace_id = $1 and mount_path = $2 and relative_path = $3 and deleted = false and document_id <> $4",
            &[
                &request.workspace_id,
                &request.mount_relative_path,
                &request.relative_path,
                &request.document_id,
            ],
        )
        .await?;
    if existing_document.is_some() {
        return Err(StoreError::conflict(
            "path_taken",
            format!(
                "document already exists at {}/{}",
                request.mount_relative_path, request.relative_path
            ),
        ));
    }

    transaction
        .execute(
            "update document_paths set mount_path = $3, relative_path = $4, manifest_version = manifest_version + 1, updated_at = now() \
             where workspace_id = $1 and document_id = $2 and deleted = false",
            &[
                &request.workspace_id,
                &request.document_id,
                &request.mount_relative_path,
                &request.relative_path,
            ],
        )
        .await?;
    let event_cursor = insert_event_tx(
        transaction,
        &request.workspace_id,
        &request.actor_id,
        Some(&request.document_id),
        Some(&request.mount_relative_path),
        Some(&request.relative_path),
        ProvenanceEventKind::DocumentMoved,
        &format!(
            "moved text document to {}",
            display_document_path(&request.mount_relative_path, &request.relative_path)
        ),
    )
    .await?;
    insert_path_revision_tx(
        transaction,
        &request.workspace_id,
        &request.document_id,
        event_cursor,
        &request.actor_id,
        &request.mount_relative_path,
        &request.relative_path,
        false,
        "document_moved",
    )
    .await?;

    Ok(())
}

fn make_document_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time before unix epoch")
        .as_nanos();
    format!("doc-{nanos}")
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time before unix epoch")
        .as_millis()
}
