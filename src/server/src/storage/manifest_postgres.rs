/**
@module PROJECTOR.SERVER.POSTGRES_MANIFEST
Owns Postgres-backed document path lifecycle rules, mount validation, and stale-cursor checks for create, move, and delete operations.
*/
// @fileimplements PROJECTOR.SERVER.POSTGRES_MANIFEST
use projector_domain::{
    CreateDocumentRequest, DeleteDocumentRequest, DocumentId, DocumentKind, MoveDocumentRequest,
    ProvenanceEventKind,
};

use super::StoreError;
use super::bodies::document_kind_db_value;
use super::body_persistence::{AsyncBodyPersistence, PostgresBodyPersistence};
use super::body_state::{BodyStateModel, FULL_TEXT_BODY_MODEL};
use super::history::insert_path_revision_tx;
use super::manifest::{display_document_path, enforce_manifest_cursor_tx, make_document_id};
use super::provenance::insert_event_tx;

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
    let body_persistence = PostgresBodyPersistence::new(transaction, &request.workspace_id);
    let initial_state = FULL_TEXT_BODY_MODEL.state_from_materialized_text(request.text.clone());
    body_persistence
        .write_current_state(document_id.as_str(), &initial_state)
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
    body_persistence
        .append_retained_history(
            event_cursor,
            &request.actor_id,
            document_id.as_str(),
            &FULL_TEXT_BODY_MODEL.created_history(&initial_state),
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
