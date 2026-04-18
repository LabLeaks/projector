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
    BootstrapSnapshot, DocumentId, DocumentKind, ManifestEntry, ManifestState,
    ProvenanceEvent, ProvenanceEventKind, RestoreDocumentBodyRevisionRequest,
    UpdateDocumentRequest,
};

use super::body_state::{
    BodyConvergenceEngine, CanonicalBodyState, RetainedBodyHistoryPayload, ThreeWayMergeBodyEngine,
    body_state_from_snapshot, upsert_body_state,
};
use super::StoreError;
use super::history::{
    FileBodyRevision, file_append_body_revision, file_read_body_revisions, insert_body_revision_tx,
};
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

    let current_state = body_state_from_snapshot(&snapshot, &document_id)
        .unwrap_or_else(|| CanonicalBodyState::full_text_merge_v1(String::new()));
    let merge = merge_text_update(&request.base_text, &current_state, &request.text);

    upsert_body_state(&mut snapshot, &document_id, merge.canonical_state());
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
    file_append_body_revision(
        state_dir,
        &request.workspace_id,
        FileBodyRevision::from_retained_history(
            event_cursor,
            event_cursor,
            request.actor_id.clone(),
            request.document_id.clone(),
            merge.retained_history(),
            now_ms(),
        ),
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

    let target_revision = file_read_body_revisions(state_dir, &request.workspace_id)?
        .into_iter()
        .find(|revision| revision.document_id == request.document_id && revision.seq == request.seq)
        .ok_or_else(|| {
            StoreError::new(format!(
                "document {} has no body revision {}",
                request.document_id, request.seq
            ))
        })?;
    let current_state = body_state_from_snapshot(&snapshot, &document_id)
        .unwrap_or_else(|| CanonicalBodyState::full_text_merge_v1(String::new()));

    snapshot.manifest.entries[entry_index].deleted = false;
    snapshot.manifest.entries[entry_index].mount_relative_path = target_mount_relative_path.clone();
    snapshot.manifest.entries[entry_index].relative_path = target_relative_path.clone();

    upsert_body_state(
        &mut snapshot,
        &document_id,
        &target_revision.materialized_body_state(),
    );

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
    file_append_body_revision(
        state_dir,
        &request.workspace_id,
        FileBodyRevision::from_retained_history(
            event_cursor,
            event_cursor,
            request.actor_id.clone(),
            request.document_id.clone(),
            &RetainedBodyHistoryPayload::full_text_revision_v1(
                current_state.materialized_text().to_owned(),
                target_revision.materialized_body_state().materialized_text().to_owned(),
                false,
            ),
            now_ms(),
        ),
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
    let current_state = transaction
        .query_opt(
            "select body_text from document_body_snapshots where workspace_id = $1 and document_id = $2",
            &[&request.workspace_id, &request.document_id],
        )
        .await?
        .map(|row| CanonicalBodyState::full_text_merge_v1(row.get::<_, String>("body_text")))
        .unwrap_or_else(|| CanonicalBodyState::full_text_merge_v1(String::new()));
    let merge = merge_text_update(&request.base_text, &current_state, &request.text);
    let merged_text = merge.canonical_state().materialized_text().to_owned();

    transaction
        .execute(
            "insert into document_body_snapshots \
             (document_id, workspace_id, body_text, compacted_through_seq) \
             values ($1, $2, $3, 0) \
             on conflict (document_id) do update set body_text = excluded.body_text, updated_at = now()",
            &[&request.document_id, &request.workspace_id, &merged_text],
        )
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
    insert_body_revision_tx(
        transaction,
        &request.workspace_id,
        &request.document_id,
        event_cursor,
        &request.actor_id,
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
    let current_state = transaction
        .query_opt(
            "select body_text from document_body_snapshots where workspace_id = $1 and document_id = $2",
            &[&request.workspace_id, &request.document_id],
        )
        .await?
        .map(|row| CanonicalBodyState::full_text_merge_v1(row.get::<_, String>("body_text")))
        .unwrap_or_else(|| CanonicalBodyState::full_text_merge_v1(String::new()));
    let target_text = transaction
        .query_opt(
            "select body_text from document_body_revisions \
             where workspace_id = $1 and document_id = $2 and seq = $3",
            &[
                &request.workspace_id,
                &request.document_id,
                &(request.seq as i64),
            ],
        )
        .await?
        .map(|row| row.get::<_, String>("body_text"))
        .ok_or_else(|| {
            StoreError::new(format!(
                "document {} has no body revision {}",
                request.document_id, request.seq
            ))
        })?;
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

    transaction
        .execute(
            "insert into document_body_snapshots \
             (document_id, workspace_id, body_text, compacted_through_seq) \
             values ($1, $2, $3, 0) \
             on conflict (document_id) do update set body_text = excluded.body_text, updated_at = now()",
            &[&request.document_id, &request.workspace_id, &target_text],
        )
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
    insert_body_revision_tx(
        transaction,
        &request.workspace_id,
        &request.document_id,
        event_cursor,
        &request.actor_id,
        &RetainedBodyHistoryPayload::full_text_revision_v1(
            current_state.materialized_text().to_owned(),
            target_text,
            false,
        ),
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

pub(crate) fn snapshot_from_rows<F>(
    rows: Vec<tokio_postgres::Row>,
    parse_kind: F,
) -> Result<BootstrapSnapshot, StoreError>
where
    F: Fn(&str) -> Result<DocumentKind, StoreError>,
{
    let mut snapshot = BootstrapSnapshot {
        manifest: ManifestState {
            entries: Vec::new(),
        },
        bodies: Vec::new(),
    };

    for row in rows {
        let document_id = DocumentId::new(row.get::<_, String>("document_id"));
        let deleted = row.get::<_, bool>("deleted");
        let kind = parse_kind(&row.get::<_, String>("kind"))?;
        snapshot.manifest.entries.push(ManifestEntry {
            document_id: document_id.clone(),
            mount_relative_path: PathBuf::from(row.get::<_, String>("mount_path")),
            relative_path: PathBuf::from(row.get::<_, String>("relative_path")),
            kind,
            deleted,
        });
        if !deleted {
            snapshot.bodies.push(
                CanonicalBodyState::full_text_merge_v1(row.get::<_, String>("body_text"))
                    .into_document_body(document_id),
            );
        }
    }

    Ok(snapshot)
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
    base: &str,
    current: &CanonicalBodyState,
    incoming: &str,
) -> MergeTextUpdate {
    MergeTextUpdate(ThreeWayMergeBodyEngine.apply_update(base, current, incoming))
}
