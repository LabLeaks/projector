/**
@module PROJECTOR.SERVER.BODIES
Owns body snapshot persistence, body read projection, and document body updates for file-backed and Postgres-backed stores.
*/
// @fileimplements PROJECTOR.SERVER.BODIES
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use projector_domain::{
    BootstrapSnapshot, DocumentId, DocumentKind, ManifestState, ProvenanceEvent,
    ProvenanceEventKind, RestoreDocumentBodyRevisionRequest, UpdateDocumentRequest,
};

use super::StoreError;
use super::body_persistence::{
    AsyncBodyPersistence, FileBodyPersistence, PostgresBodyPersistence, SnapshotBodyPersistence,
};
use super::body_state::{
    BodyConvergenceEngine, BodyStateModel, CanonicalBodyState, FULL_TEXT_BODY_MODEL,
    RetainedBodyHistoryKind, RetainedBodyHistoryPayload, YrsConvergenceBodyEngine,
};
use super::history::{file_read_body_revisions, replay_body_revision_run};
use super::provenance::{file_append_workspace_event, insert_event_tx};
use super::workspaces::workspace_dir;

pub(crate) fn file_persist_workspace_snapshot(
    state_dir: &Path,
    workspace_id: &str,
    snapshot: &BootstrapSnapshot,
) -> Result<(), StoreError> {
    let workspace_dir = workspace_dir(state_dir, workspace_id);
    fs::create_dir_all(&workspace_dir)?;
    let snapshot_path = workspace_dir.join("snapshot.json");
    let encoded =
        serde_json::to_vec_pretty(snapshot).map_err(|err| StoreError::new(err.to_string()))?;
    fs::write(snapshot_path, encoded)?;
    Ok(())
}

pub(crate) fn file_read_workspace_snapshot(
    state_dir: &Path,
    workspace_id: &str,
) -> Result<BootstrapSnapshot, StoreError> {
    let snapshot_path = workspace_dir(state_dir, workspace_id).join("snapshot.json");
    if !snapshot_path.exists() {
        return Ok(BootstrapSnapshot::default());
    }

    let content = fs::read(snapshot_path)?;
    serde_json::from_slice(&content).map_err(|err| StoreError::new(err.to_string()))
}

pub(crate) fn file_update_document(
    state_dir: &Path,
    request: &UpdateDocumentRequest,
) -> Result<(), StoreError> {
    let mut snapshot = file_read_workspace_snapshot(state_dir, &request.workspace_id)?;
    let document_id = DocumentId::new(&request.document_id);
    if !snapshot
        .manifest
        .entries
        .iter()
        .any(|entry| !entry.deleted && entry.document_id == document_id)
    {
        return Err(StoreError::new(format!(
            "document {} is not live in workspace {}",
            request.document_id, request.workspace_id
        )));
    }

    let body_persistence = FileBodyPersistence::new(state_dir, &request.workspace_id);
    let current_state = body_persistence.load_current_state(&snapshot, &document_id);
    let merge = merge_text_update(
        &request.actor_id,
        &request.base_text,
        &current_state,
        &request.text,
    );

    body_persistence.write_current_state(&mut snapshot, &document_id, merge.canonical_state());
    let Some(entry) = snapshot
        .manifest
        .entries
        .iter()
        .find(|entry| !entry.deleted && entry.document_id == document_id)
    else {
        return Err(StoreError::new(format!(
            "document {} is not live in workspace {}",
            request.document_id, request.workspace_id
        )));
    };
    file_persist_workspace_snapshot(state_dir, &request.workspace_id, &snapshot)?;
    let event_cursor =
        super::provenance::file_workspace_cursor(state_dir, &request.workspace_id)? + 1;
    file_append_workspace_event(
        state_dir,
        &request.workspace_id,
        ProvenanceEvent {
            cursor: event_cursor,
            timestamp_ms: now_ms(),
            actor_id: projector_domain::ActorId::new(request.actor_id.clone()),
            document_id: Some(document_id),
            mount_relative_path: Some(entry.mount_relative_path.display().to_string()),
            relative_path: Some(entry.relative_path.display().to_string()),
            summary: merge.summary_for_path(&entry.mount_relative_path, &entry.relative_path),
            kind: ProvenanceEventKind::DocumentUpdated,
        },
    )?;
    body_persistence.append_retained_history(
        event_cursor,
        &request.actor_id,
        &request.document_id,
        merge.retained_history(),
        now_ms(),
    )?;
    Ok(())
}

pub(crate) fn file_restore_document_body_revision(
    state_dir: &Path,
    request: &RestoreDocumentBodyRevisionRequest,
) -> Result<(), StoreError> {
    let mut snapshot = file_read_workspace_snapshot(state_dir, &request.workspace_id)?;
    let document_id = DocumentId::new(&request.document_id);
    let Some(entry_index) = snapshot
        .manifest
        .entries
        .iter()
        .position(|entry| entry.document_id == document_id)
    else {
        return Err(StoreError::new(format!(
            "document {} is not present in workspace {}",
            request.document_id, request.workspace_id
        )));
    };
    let entry = snapshot.manifest.entries[entry_index].clone();
    let target_mount_relative_path = request
        .target_mount_relative_path
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| entry.mount_relative_path.clone());
    let target_relative_path = request
        .target_relative_path
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| entry.relative_path.clone());

    if (entry.deleted
        || target_mount_relative_path != entry.mount_relative_path
        || target_relative_path != entry.relative_path)
        && snapshot.manifest.entries.iter().any(|candidate| {
            candidate.document_id != document_id
                && !candidate.deleted
                && candidate.mount_relative_path == target_mount_relative_path
                && candidate.relative_path == target_relative_path
        })
    {
        return Err(StoreError::conflict(
            "path_taken",
            format!(
                "document already exists at {}/{}",
                target_mount_relative_path.display(),
                target_relative_path.display()
            ),
        ));
    }

    let body_revisions = file_read_body_revisions(state_dir, &request.workspace_id)?;
    let target_revision = body_revisions
        .iter()
        .find(|revision| revision.document_id == request.document_id && revision.seq == request.seq)
        .ok_or_else(|| {
            StoreError::new(format!(
                "document {} has no body revision {}",
                request.document_id, request.seq
            ))
        })?;
    let fallback_target_state = target_revision.materialized_body_state();
    let body_persistence = FileBodyPersistence::new(state_dir, &request.workspace_id);
    let current_state = body_persistence.load_current_state(&snapshot, &document_id);

    snapshot.manifest.entries[entry_index].deleted = false;
    snapshot.manifest.entries[entry_index].mount_relative_path = target_mount_relative_path.clone();
    snapshot.manifest.entries[entry_index].relative_path = target_relative_path.clone();

    let replayed_target_state =
        replay_body_revision_run(body_revisions.into_iter().filter(|revision| {
            revision.document_id == request.document_id && revision.seq <= request.seq
        }))
        .remove(request.document_id.as_str())
        .unwrap_or_else(|| fallback_target_state.clone());
    let target_state =
        if replayed_target_state.materialized_text() == fallback_target_state.materialized_text() {
            replayed_target_state
        } else {
            fallback_target_state
        };
    body_persistence.write_current_state(&mut snapshot, &document_id, &target_state);

    file_persist_workspace_snapshot(state_dir, &request.workspace_id, &snapshot)?;
    let event_cursor =
        super::provenance::file_workspace_cursor(state_dir, &request.workspace_id)? + 1;
    file_append_workspace_event(
        state_dir,
        &request.workspace_id,
        ProvenanceEvent {
            cursor: event_cursor,
            timestamp_ms: now_ms(),
            actor_id: projector_domain::ActorId::new(request.actor_id.clone()),
            document_id: Some(document_id.clone()),
            mount_relative_path: Some(target_mount_relative_path.display().to_string()),
            relative_path: Some(target_relative_path.display().to_string()),
            summary: format!(
                "restored text document at {}/{} from body revision {}",
                target_mount_relative_path.display(),
                target_relative_path.display(),
                request.seq
            ),
            kind: ProvenanceEventKind::DocumentUpdated,
        },
    )?;
    body_persistence.append_retained_history(
        event_cursor,
        &request.actor_id,
        &request.document_id,
        &FULL_TEXT_BODY_MODEL.restored_history(&current_state, &target_state),
        now_ms(),
    )?;
    if entry.deleted
        || target_mount_relative_path != entry.mount_relative_path
        || target_relative_path != entry.relative_path
    {
        super::history::file_append_path_revision(
            state_dir,
            &request.workspace_id,
            super::history::FilePathRevision {
                seq: event_cursor,
                workspace_cursor: event_cursor,
                actor_id: request.actor_id.clone(),
                document_id: request.document_id.clone(),
                mount_path: target_mount_relative_path.display().to_string(),
                relative_path: target_relative_path.display().to_string(),
                deleted: false,
                event_kind: "document_restored".to_owned(),
                timestamp_ms: now_ms(),
            },
        )?;
    }
    Ok(())
}

pub(crate) async fn postgres_update_document(
    transaction: &tokio_postgres::Transaction<'_>,
    request: &UpdateDocumentRequest,
) -> Result<(), StoreError> {
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
    let body_persistence = PostgresBodyPersistence::new(transaction, &request.workspace_id);
    let current_state = body_persistence
        .load_current_state(&request.document_id)
        .await?;
    let merge = merge_text_update(
        &request.actor_id,
        &request.base_text,
        &current_state,
        &request.text,
    );
    body_persistence
        .write_current_state(&request.document_id, merge.canonical_state())
        .await?;
    let event_cursor = insert_event_tx(
        transaction,
        &request.workspace_id,
        &request.actor_id,
        Some(&request.document_id),
        Some(&mount_path),
        Some(&relative_path),
        ProvenanceEventKind::DocumentUpdated,
        &merge.summary_for_path(Path::new(&mount_path), Path::new(&relative_path)),
    )
    .await?;
    body_persistence
        .append_retained_history(
            event_cursor,
            &request.actor_id,
            &request.document_id,
            merge.retained_history(),
        )
        .await?;

    Ok(())
}

pub(crate) async fn postgres_restore_document_body_revision(
    transaction: &tokio_postgres::Transaction<'_>,
    request: &RestoreDocumentBodyRevisionRequest,
) -> Result<(), StoreError> {
    let path_row = transaction
        .query_opt(
            "select mount_path, relative_path, deleted from document_paths \
             where workspace_id = $1 and document_id = $2",
            &[&request.workspace_id, &request.document_id],
        )
        .await?;
    let Some(path_row) = path_row else {
        return Err(StoreError::new(format!(
            "document {} is not present in workspace {}",
            request.document_id, request.workspace_id
        )));
    };
    let mount_path = path_row.get::<_, String>("mount_path");
    let relative_path = path_row.get::<_, String>("relative_path");
    let was_deleted = path_row.get::<_, bool>("deleted");
    let target_mount_path = request
        .target_mount_relative_path
        .as_deref()
        .unwrap_or(&mount_path);
    let target_relative_path = request
        .target_relative_path
        .as_deref()
        .unwrap_or(&relative_path);
    if was_deleted {
        let live_path_conflict = transaction
            .query_opt(
                "select document_id from document_paths \
                 where workspace_id = $1 and mount_path = $2 and relative_path = $3 and deleted = false and document_id <> $4",
                &[
                    &request.workspace_id,
                    &target_mount_path,
                    &target_relative_path,
                    &request.document_id,
                ],
            )
            .await?;
        if live_path_conflict.is_some() {
            return Err(StoreError::conflict(
                "path_taken",
                format!("document already exists at {target_mount_path}/{target_relative_path}"),
            ));
        }
    }
    if !was_deleted && (target_mount_path != mount_path || target_relative_path != relative_path) {
        let live_path_conflict = transaction
            .query_opt(
                "select document_id from document_paths \
                 where workspace_id = $1 and mount_path = $2 and relative_path = $3 and deleted = false and document_id <> $4",
                &[
                    &request.workspace_id,
                    &target_mount_path,
                    &target_relative_path,
                    &request.document_id,
                ],
            )
            .await?;
        if live_path_conflict.is_some() {
            return Err(StoreError::conflict(
                "path_taken",
                format!("document already exists at {target_mount_path}/{target_relative_path}"),
            ));
        }
    }
    let body_persistence = PostgresBodyPersistence::new(transaction, &request.workspace_id);
    let current_state = body_persistence
        .load_current_state(&request.document_id)
        .await?;
    let target_rows = transaction
        .query(
            "select seq, workspace_cursor, checkpoint_anchor_seq, history_kind, base_text, body_text, conflicted \
             from document_body_revisions \
             where workspace_id = $1 and document_id = $2 and seq <= $3 \
             order by seq asc",
            &[
                &request.workspace_id,
                &request.document_id,
                &(request.seq as i64),
            ],
        )
        .await?;
    if target_rows.is_empty() {
        return Err(StoreError::new(format!(
            "document {} has no body revision {}",
            request.document_id, request.seq
        )));
    }
    if was_deleted || target_mount_path != mount_path || target_relative_path != relative_path {
        transaction
            .execute(
                "update document_paths set mount_path = $3, relative_path = $4, deleted = false, updated_at = now() \
                 where workspace_id = $1 and document_id = $2",
                &[
                    &request.workspace_id,
                    &request.document_id,
                    &target_mount_path,
                    &target_relative_path,
                ],
            )
            .await?;
    }

    let target_revisions = target_rows
        .into_iter()
        .map(|row| {
            let history_kind =
                RetainedBodyHistoryKind::parse(row.get::<_, String>("history_kind").as_str())
                    .expect("stored retained body history kind should parse");
            super::history::FileBodyRevision {
                seq: row.get::<_, i64>("seq") as u64,
                workspace_cursor: row.get::<_, i64>("workspace_cursor") as u64,
                actor_id: String::new(),
                document_id: request.document_id.clone(),
                checkpoint_anchor_seq: row
                    .get::<_, Option<i64>>("checkpoint_anchor_seq")
                    .map(|seq| seq as u64),
                history_kind,
                base_text: row.get::<_, String>("base_text"),
                body_text: row.get::<_, String>("body_text"),
                conflicted: row.get::<_, bool>("conflicted"),
                timestamp_ms: 0,
            }
        })
        .collect::<Vec<_>>();
    let fallback_target_state = target_revisions
        .last()
        .expect("target rows should contain requested revision")
        .materialized_body_state();
    let replayed_target_state = replay_body_revision_run(target_revisions.into_iter())
        .remove(request.document_id.as_str())
        .ok_or_else(|| {
            StoreError::new(format!(
                "document {} has no body revision {}",
                request.document_id, request.seq
            ))
        })?;
    let target_state =
        if replayed_target_state.materialized_text() == fallback_target_state.materialized_text() {
            replayed_target_state
        } else {
            fallback_target_state
        };
    body_persistence
        .write_current_state(&request.document_id, &target_state)
        .await?;
    let event_cursor = insert_event_tx(
        transaction,
        &request.workspace_id,
        &request.actor_id,
        Some(&request.document_id),
        Some(target_mount_path),
        Some(target_relative_path),
        ProvenanceEventKind::DocumentUpdated,
        &format!(
            "restored text document at {target_mount_path}/{target_relative_path} from body revision {}",
            request.seq
        ),
    )
    .await?;
    body_persistence
        .append_retained_history(
            event_cursor,
            &request.actor_id,
            &request.document_id,
            &FULL_TEXT_BODY_MODEL.restored_history(&current_state, &target_state),
        )
        .await?;
    if was_deleted || target_mount_path != mount_path || target_relative_path != relative_path {
        super::history::insert_path_revision_tx(
            transaction,
            &request.workspace_id,
            &request.document_id,
            event_cursor,
            &request.actor_id,
            target_mount_path,
            target_relative_path,
            false,
            "document_restored",
        )
        .await?;
    }

    Ok(())
}

pub(crate) fn parse_document_kind(raw: &str) -> Result<DocumentKind, StoreError> {
    match raw {
        "text" | "markdown" => Ok(DocumentKind::Text),
        other => Err(StoreError::new(format!("unknown document kind {other}"))),
    }
}

pub(crate) fn document_kind_db_value(kind: &DocumentKind) -> &'static str {
    match kind {
        DocumentKind::Text => "text",
    }
}

pub(crate) fn snapshot_subset_for_documents(
    snapshot: &BootstrapSnapshot,
    document_ids: &HashSet<DocumentId>,
) -> BootstrapSnapshot {
    let manifest = snapshot
        .manifest
        .entries
        .iter()
        .filter(|entry| document_ids.contains(&entry.document_id))
        .cloned()
        .collect::<Vec<_>>();
    let bodies = snapshot
        .bodies
        .iter()
        .filter(|body| document_ids.contains(&body.document_id))
        .cloned()
        .collect::<Vec<_>>();

    BootstrapSnapshot {
        manifest: ManifestState { entries: manifest },
        bodies,
    }
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time before unix epoch")
        .as_millis()
}

pub(crate) struct MergeTextUpdate(super::body_state::BodyConvergenceResult);

impl MergeTextUpdate {
    pub(crate) fn canonical_state(&self) -> &CanonicalBodyState {
        self.0.canonical_state()
    }

    pub(crate) fn retained_history(&self) -> &RetainedBodyHistoryPayload {
        self.0.retained_history()
    }

    pub(crate) fn summary_for_path(&self, mount_path: &Path, relative_path: &Path) -> String {
        self.0.summary_for_path(mount_path, relative_path)
    }
}

pub(crate) fn merge_text_update(
    actor_id: &str,
    base: &str,
    current: &CanonicalBodyState,
    incoming: &str,
) -> MergeTextUpdate {
    MergeTextUpdate(YrsConvergenceBodyEngine.apply_update(actor_id, base, current, incoming))
}
