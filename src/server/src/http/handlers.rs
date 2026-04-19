/**
@module PROJECTOR.SERVER.HTTP_HANDLERS
Defines the projector HTTP routes, request handlers, and store-error mapping for bootstrap, delta reads, document lifecycle writes, and event listing.
*/
// @fileimplements PROJECTOR.SERVER.HTTP_HANDLERS
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use projector_domain::{
    ApiErrorResponse, BootstrapRequest, BootstrapResponse, ChangesSinceRequest,
    ChangesSinceResponse, CreateDocumentRequest, CreateDocumentResponse, DeleteDocumentRequest,
    ListBodyRevisionsRequest, ListBodyRevisionsResponse, ListEventsRequest, ListEventsResponse,
    ListPathRevisionsRequest, ListPathRevisionsResponse, ListSyncEntriesRequest,
    ListSyncEntriesResponse, MoveDocumentRequest, PreviewRedactDocumentBodyHistoryRequest,
    PreviewRedactDocumentBodyHistoryResponse, PurgeDocumentBodyHistoryRequest,
    ReconstructWorkspaceRequest, ReconstructWorkspaceResponse, RedactDocumentBodyHistoryRequest,
    ResolveHistoricalPathRequest, ResolveHistoricalPathResponse,
    RestoreDocumentBodyRevisionRequest, RestoreWorkspaceRequest, UpdateDocumentRequest,
};

use crate::{StoreError, WorkspaceStore};

#[derive(Clone)]
struct AppState {
    store: Arc<dyn WorkspaceStore>,
}

pub(super) fn app(store: Arc<dyn WorkspaceStore>) -> Router {
    Router::new()
        .route("/bootstrap", post(bootstrap))
        .route("/sync-entries/list", post(list_sync_entries))
        .route("/changes/since", post(changes_since))
        .route("/documents/create", post(create_document))
        .route("/documents/update", post(update_document))
        .route("/documents/delete", post(delete_document))
        .route("/documents/move", post(move_document))
        .route("/events/list", post(list_events))
        .route("/history/body/list", post(list_body_revisions))
        .route("/history/body/restore", post(restore_body_revision))
        .route(
            "/history/body/redact/preview",
            post(preview_redact_body_history),
        )
        .route("/history/body/redact", post(redact_body_history))
        .route("/history/body/purge", post(purge_body_history))
        .route("/history/path/list", post(list_path_revisions))
        .route(
            "/history/workspace/reconstruct",
            post(reconstruct_workspace),
        )
        .route("/history/workspace/restore", post(restore_workspace))
        .route("/history/path/resolve", post(resolve_historical_path))
        .with_state(AppState { store })
}

async fn bootstrap(
    State(state): State<AppState>,
    Json(request): Json<BootstrapRequest>,
) -> Result<Json<BootstrapResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    let mounts = request
        .projection_relative_paths
        .iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    let (snapshot, cursor) = state
        .store
        .bootstrap_workspace(
            &request.workspace_id,
            &mounts,
            request.source_repo_name.as_deref(),
            request.sync_entry_kind,
        )
        .await
        .map_err(store_error_response)?;

    Ok(Json(BootstrapResponse { snapshot, cursor }))
}

async fn list_sync_entries(
    State(state): State<AppState>,
    Json(request): Json<ListSyncEntriesRequest>,
) -> Result<Json<ListSyncEntriesResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    let entries = state
        .store
        .list_sync_entries(request.limit)
        .await
        .map_err(store_error_response)?;

    Ok(Json(ListSyncEntriesResponse { entries }))
}

async fn changes_since(
    State(state): State<AppState>,
    Json(request): Json<ChangesSinceRequest>,
) -> Result<Json<ChangesSinceResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    let (snapshot, cursor) = state
        .store
        .changes_since(&request.workspace_id, request.since_cursor)
        .await
        .map_err(store_error_response)?;

    Ok(Json(ChangesSinceResponse { snapshot, cursor }))
}

async fn create_document(
    State(state): State<AppState>,
    Json(request): Json<CreateDocumentRequest>,
) -> Result<Json<CreateDocumentResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    let document_id = state
        .store
        .create_document(&request)
        .await
        .map_err(store_error_response)?;

    Ok(Json(CreateDocumentResponse {
        document_id: document_id.as_str().to_owned(),
    }))
}

async fn update_document(
    State(state): State<AppState>,
    Json(request): Json<UpdateDocumentRequest>,
) -> Result<StatusCode, (StatusCode, Json<ApiErrorResponse>)> {
    state
        .store
        .update_document(&request)
        .await
        .map_err(store_error_response)?;

    Ok(StatusCode::NO_CONTENT)
}

async fn delete_document(
    State(state): State<AppState>,
    Json(request): Json<DeleteDocumentRequest>,
) -> Result<StatusCode, (StatusCode, Json<ApiErrorResponse>)> {
    state
        .store
        .delete_document(&request)
        .await
        .map_err(store_error_response)?;

    Ok(StatusCode::NO_CONTENT)
}

async fn move_document(
    State(state): State<AppState>,
    Json(request): Json<MoveDocumentRequest>,
) -> Result<StatusCode, (StatusCode, Json<ApiErrorResponse>)> {
    state
        .store
        .move_document(&request)
        .await
        .map_err(store_error_response)?;

    Ok(StatusCode::NO_CONTENT)
}

async fn list_events(
    State(state): State<AppState>,
    Json(request): Json<ListEventsRequest>,
) -> Result<Json<ListEventsResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    let events = state
        .store
        .list_events(&request.workspace_id, request.limit)
        .await
        .map_err(store_error_response)?;

    Ok(Json(ListEventsResponse { events }))
}

async fn list_body_revisions(
    State(state): State<AppState>,
    Json(request): Json<ListBodyRevisionsRequest>,
) -> Result<Json<ListBodyRevisionsResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    let revisions = state
        .store
        .list_body_revisions(&request.workspace_id, &request.document_id, request.limit)
        .await
        .map_err(store_error_response)?;

    Ok(Json(ListBodyRevisionsResponse { revisions }))
}

async fn list_path_revisions(
    State(state): State<AppState>,
    Json(request): Json<ListPathRevisionsRequest>,
) -> Result<Json<ListPathRevisionsResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    let revisions = state
        .store
        .list_path_revisions(&request.workspace_id, &request.document_id, request.limit)
        .await
        .map_err(store_error_response)?;

    Ok(Json(ListPathRevisionsResponse { revisions }))
}

async fn reconstruct_workspace(
    State(state): State<AppState>,
    Json(request): Json<ReconstructWorkspaceRequest>,
) -> Result<Json<ReconstructWorkspaceResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    let snapshot = state
        .store
        .reconstruct_workspace_at_cursor(&request.workspace_id, request.cursor)
        .await
        .map_err(store_error_response)?;

    Ok(Json(ReconstructWorkspaceResponse { snapshot }))
}

async fn restore_workspace(
    State(state): State<AppState>,
    Json(request): Json<RestoreWorkspaceRequest>,
) -> Result<StatusCode, (StatusCode, Json<ApiErrorResponse>)> {
    state
        .store
        .restore_workspace_at_cursor(&request)
        .await
        .map_err(store_error_response)?;

    Ok(StatusCode::NO_CONTENT)
}

async fn restore_body_revision(
    State(state): State<AppState>,
    Json(request): Json<RestoreDocumentBodyRevisionRequest>,
) -> Result<StatusCode, (StatusCode, Json<ApiErrorResponse>)> {
    state
        .store
        .restore_document_body_revision(&request)
        .await
        .map_err(store_error_response)?;

    Ok(StatusCode::NO_CONTENT)
}

async fn preview_redact_body_history(
    State(state): State<AppState>,
    Json(request): Json<PreviewRedactDocumentBodyHistoryRequest>,
) -> Result<Json<PreviewRedactDocumentBodyHistoryResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    let matches = state
        .store
        .preview_redact_document_body_history(&request)
        .await
        .map_err(store_error_response)?;

    Ok(Json(PreviewRedactDocumentBodyHistoryResponse { matches }))
}

async fn purge_body_history(
    State(state): State<AppState>,
    Json(request): Json<PurgeDocumentBodyHistoryRequest>,
) -> Result<StatusCode, (StatusCode, Json<ApiErrorResponse>)> {
    state
        .store
        .purge_document_body_history(&request)
        .await
        .map_err(store_error_response)?;

    Ok(StatusCode::NO_CONTENT)
}

async fn redact_body_history(
    State(state): State<AppState>,
    Json(request): Json<RedactDocumentBodyHistoryRequest>,
) -> Result<StatusCode, (StatusCode, Json<ApiErrorResponse>)> {
    state
        .store
        .redact_document_body_history(&request)
        .await
        .map_err(store_error_response)?;

    Ok(StatusCode::NO_CONTENT)
}

async fn resolve_historical_path(
    State(state): State<AppState>,
    Json(request): Json<ResolveHistoricalPathRequest>,
) -> Result<Json<ResolveHistoricalPathResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    let document_id = state
        .store
        .resolve_document_by_historical_path(&request)
        .await
        .map_err(store_error_response)?;

    Ok(Json(ResolveHistoricalPathResponse {
        document_id: document_id.as_str().to_owned(),
    }))
}

fn store_error_response(err: StoreError) -> (StatusCode, Json<ApiErrorResponse>) {
    let status = if err.is_conflict() {
        StatusCode::CONFLICT
    } else {
        StatusCode::BAD_REQUEST
    };
    (
        status,
        Json(ApiErrorResponse {
            code: err.code().to_owned(),
            message: err.to_string(),
        }),
    )
}
