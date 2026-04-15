/**
@module PROJECTOR.SERVER.HISTORY
Owns durable document body-revision and path-revision capture for file-backed and Postgres-backed stores so future restore workflows have explicit history beyond current state and provenance summaries.
*/
// @fileimplements PROJECTOR.SERVER.HISTORY
use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use tokio_postgres::Client;

use super::StoreError;
use super::bodies::{file_persist_workspace_snapshot, file_read_workspace_snapshot};
use super::provenance::{
    current_workspace_cursor_tx, file_append_workspace_event, file_workspace_cursor,
    insert_event_tx,
};
use super::workspaces::workspace_dir;
use projector_domain::{
    BootstrapSnapshot, DocumentBody, DocumentBodyRevision, DocumentId, DocumentKind,
    DocumentPathRevision, ManifestEntry, ManifestState, ProvenanceEvent, ProvenanceEventKind,
    RestoreWorkspaceRequest,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct FileBodyRevision {
    pub seq: u64,
    #[serde(default)]
    pub workspace_cursor: u64,
    pub actor_id: String,
    pub document_id: String,
    pub base_text: String,
    pub body_text: String,
    pub conflicted: bool,
    pub timestamp_ms: u128,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct FilePathRevision {
    pub seq: u64,
    #[serde(default)]
    pub workspace_cursor: u64,
    pub actor_id: String,
    pub document_id: String,
    pub mount_path: String,
    pub relative_path: String,
    pub deleted: bool,
    pub event_kind: String,
    pub timestamp_ms: u128,
}

fn file_body_revisions_path(state_dir: &Path, workspace_id: &str) -> std::path::PathBuf {
    workspace_dir(state_dir, workspace_id).join("body_revisions.json")
}

fn effective_workspace_cursor(seq: u64, workspace_cursor: u64) -> u64 {
    if workspace_cursor == 0 {
        seq
    } else {
        workspace_cursor
    }
}

fn file_path_history_path(state_dir: &Path, workspace_id: &str) -> std::path::PathBuf {
    workspace_dir(state_dir, workspace_id).join("path_history.json")
}

pub(crate) fn file_read_body_revisions(
    state_dir: &Path,
    workspace_id: &str,
) -> Result<Vec<FileBodyRevision>, StoreError> {
    let path = file_body_revisions_path(state_dir, workspace_id);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read(path)?;
    serde_json::from_slice(&content).map_err(|err| StoreError::new(err.to_string()))
}

pub(crate) fn file_append_body_revision(
    state_dir: &Path,
    workspace_id: &str,
    revision: FileBodyRevision,
) -> Result<(), StoreError> {
    let workspace_root = workspace_dir(state_dir, workspace_id);
    fs::create_dir_all(&workspace_root)?;
    let path = file_body_revisions_path(state_dir, workspace_id);
    let mut revisions = file_read_body_revisions(state_dir, workspace_id)?;
    revisions.push(revision);
    let encoded =
        serde_json::to_vec_pretty(&revisions).map_err(|err| StoreError::new(err.to_string()))?;
    fs::write(path, encoded)?;
    Ok(())
}

pub(crate) fn file_list_body_revisions(
    state_dir: &Path,
    workspace_id: &str,
    document_id: &str,
    limit: usize,
) -> Result<Vec<DocumentBodyRevision>, StoreError> {
    let mut revisions = file_read_body_revisions(state_dir, workspace_id)?
        .into_iter()
        .filter(|revision| revision.document_id == document_id)
        .map(|revision| DocumentBodyRevision {
            seq: revision.seq,
            actor_id: revision.actor_id,
            document_id: revision.document_id,
            base_text: revision.base_text,
            body_text: revision.body_text,
            conflicted: revision.conflicted,
            timestamp_ms: revision.timestamp_ms,
        })
        .collect::<Vec<_>>();
    if revisions.len() > limit {
        revisions = revisions.split_off(revisions.len() - limit);
    }
    Ok(revisions)
}

pub(crate) fn file_read_path_history(
    state_dir: &Path,
    workspace_id: &str,
) -> Result<Vec<FilePathRevision>, StoreError> {
    let path = file_path_history_path(state_dir, workspace_id);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read(path)?;
    serde_json::from_slice(&content).map_err(|err| StoreError::new(err.to_string()))
}

pub(crate) fn file_append_path_revision(
    state_dir: &Path,
    workspace_id: &str,
    revision: FilePathRevision,
) -> Result<(), StoreError> {
    let workspace_root = workspace_dir(state_dir, workspace_id);
    fs::create_dir_all(&workspace_root)?;
    let path = file_path_history_path(state_dir, workspace_id);
    let mut revisions = file_read_path_history(state_dir, workspace_id)?;
    revisions.push(revision);
    let encoded =
        serde_json::to_vec_pretty(&revisions).map_err(|err| StoreError::new(err.to_string()))?;
    fs::write(path, encoded)?;
    Ok(())
}

pub(crate) fn file_list_path_revisions(
    state_dir: &Path,
    workspace_id: &str,
    document_id: &str,
    limit: usize,
) -> Result<Vec<DocumentPathRevision>, StoreError> {
    let mut revisions = file_read_path_history(state_dir, workspace_id)?
        .into_iter()
        .filter(|revision| revision.document_id == document_id)
        .map(|revision| DocumentPathRevision {
            seq: revision.seq,
            actor_id: revision.actor_id,
            document_id: revision.document_id,
            mount_path: revision.mount_path,
            relative_path: revision.relative_path,
            deleted: revision.deleted,
            event_kind: revision.event_kind,
            timestamp_ms: revision.timestamp_ms,
        })
        .collect::<Vec<_>>();
    if revisions.len() > limit {
        revisions = revisions.split_off(revisions.len() - limit);
    }
    Ok(revisions)
}

pub(crate) fn file_resolve_document_by_historical_path(
    state_dir: &Path,
    workspace_id: &str,
    mount_path: &str,
    relative_path: &str,
) -> Result<DocumentId, StoreError> {
    file_read_path_history(state_dir, workspace_id)?
        .into_iter()
        .filter(|revision| {
            revision.mount_path == mount_path && revision.relative_path == relative_path
        })
        .max_by_key(|revision| revision.seq)
        .map(|revision| DocumentId::new(revision.document_id))
        .ok_or_else(|| {
            StoreError::new(format!(
                "no document path history found at {mount_path}/{relative_path}"
            ))
        })
}

pub(crate) fn file_reconstruct_workspace_at_cursor(
    state_dir: &Path,
    workspace_id: &str,
    cursor: u64,
) -> Result<BootstrapSnapshot, StoreError> {
    let path_history = file_read_path_history(state_dir, workspace_id)?;
    let body_history = file_read_body_revisions(state_dir, workspace_id)?;

    let latest_paths = path_history
        .into_iter()
        .filter(|revision| {
            effective_workspace_cursor(revision.seq, revision.workspace_cursor) <= cursor
        })
        .fold(
            HashMap::<String, FilePathRevision>::new(),
            |mut acc, revision| {
                let replace = acc
                    .get(&revision.document_id)
                    .map(|current| {
                        effective_workspace_cursor(revision.seq, revision.workspace_cursor)
                            > effective_workspace_cursor(current.seq, current.workspace_cursor)
                            || (effective_workspace_cursor(revision.seq, revision.workspace_cursor)
                                == effective_workspace_cursor(
                                    current.seq,
                                    current.workspace_cursor,
                                )
                                && revision.seq > current.seq)
                    })
                    .unwrap_or(true);
                if replace {
                    acc.insert(revision.document_id.clone(), revision);
                }
                acc
            },
        );

    let latest_bodies = body_history
        .into_iter()
        .filter(|revision| {
            effective_workspace_cursor(revision.seq, revision.workspace_cursor) <= cursor
        })
        .fold(
            HashMap::<String, FileBodyRevision>::new(),
            |mut acc, revision| {
                let replace = acc
                    .get(&revision.document_id)
                    .map(|current| {
                        effective_workspace_cursor(revision.seq, revision.workspace_cursor)
                            > effective_workspace_cursor(current.seq, current.workspace_cursor)
                            || (effective_workspace_cursor(revision.seq, revision.workspace_cursor)
                                == effective_workspace_cursor(
                                    current.seq,
                                    current.workspace_cursor,
                                )
                                && revision.seq > current.seq)
                    })
                    .unwrap_or(true);
                if replace {
                    acc.insert(revision.document_id.clone(), revision);
                }
                acc
            },
        );

    let mut entries = latest_paths
        .into_values()
        .map(|revision| ManifestEntry {
            document_id: DocumentId::new(revision.document_id),
            mount_relative_path: revision.mount_path.into(),
            relative_path: revision.relative_path.into(),
            kind: DocumentKind::Text,
            deleted: revision.deleted,
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        left.mount_relative_path
            .cmp(&right.mount_relative_path)
            .then_with(|| left.relative_path.cmp(&right.relative_path))
            .then_with(|| left.document_id.as_str().cmp(right.document_id.as_str()))
    });

    let mut bodies = entries
        .iter()
        .filter(|entry| !entry.deleted)
        .filter_map(|entry| {
            latest_bodies
                .get(entry.document_id.as_str())
                .map(|revision| DocumentBody {
                    document_id: entry.document_id.clone(),
                    text: revision.body_text.clone(),
                })
        })
        .collect::<Vec<_>>();
    bodies.sort_by(|left, right| left.document_id.as_str().cmp(right.document_id.as_str()));

    Ok(BootstrapSnapshot {
        manifest: ManifestState { entries },
        bodies,
    })
}

pub(crate) fn file_restore_workspace_at_cursor(
    state_dir: &Path,
    request: &RestoreWorkspaceRequest,
) -> Result<(), StoreError> {
    let current_cursor = file_workspace_cursor(state_dir, &request.workspace_id)?;
    let provided_cursor = request
        .based_on_cursor
        .ok_or_else(|| StoreError::new("workspace restore missing based_on_cursor precondition"))?;
    if provided_cursor != current_cursor {
        return Err(StoreError::conflict(
            "stale_cursor",
            format!(
                "workspace restore based on stale cursor {provided_cursor}; current workspace cursor is {current_cursor}"
            ),
        ));
    }
    if request.cursor > current_cursor {
        return Err(StoreError::new(format!(
            "workspace restore target cursor {} is newer than current workspace cursor {}",
            request.cursor, current_cursor
        )));
    }

    let current_snapshot = file_read_workspace_snapshot(state_dir, &request.workspace_id)?;
    let target_snapshot =
        file_reconstruct_workspace_at_cursor(state_dir, &request.workspace_id, request.cursor)?;
    let restored_snapshot =
        build_restored_live_workspace_snapshot(&current_snapshot, &target_snapshot);
    let changes =
        diff_workspace_restore_changes(&current_snapshot, &restored_snapshot, request.cursor);

    file_persist_workspace_snapshot(state_dir, &request.workspace_id, &restored_snapshot)?;
    for change in changes {
        let event_cursor = file_workspace_cursor(state_dir, &request.workspace_id)? + 1;
        file_append_workspace_event(
            state_dir,
            &request.workspace_id,
            ProvenanceEvent {
                cursor: event_cursor,
                timestamp_ms: current_time_ms(),
                actor_id: projector_domain::ActorId::new(request.actor_id.clone()),
                document_id: Some(change.document_id.clone()),
                mount_relative_path: Some(change.path.mount_path.clone()),
                relative_path: Some(change.path.relative_path.clone()),
                summary: change.summary.clone(),
                kind: change.kind.clone(),
            },
        )?;
        if let Some(body) = change.body {
            file_append_body_revision(
                state_dir,
                &request.workspace_id,
                FileBodyRevision {
                    seq: event_cursor,
                    workspace_cursor: event_cursor,
                    actor_id: request.actor_id.clone(),
                    document_id: change.document_id.as_str().to_owned(),
                    base_text: body.base_text,
                    body_text: body.body_text,
                    conflicted: false,
                    timestamp_ms: current_time_ms(),
                },
            )?;
        }
        file_append_path_revision(
            state_dir,
            &request.workspace_id,
            FilePathRevision {
                seq: event_cursor,
                workspace_cursor: event_cursor,
                actor_id: request.actor_id.clone(),
                document_id: change.document_id.as_str().to_owned(),
                mount_path: change.path.mount_path,
                relative_path: change.path.relative_path,
                deleted: change.path.deleted,
                event_kind: change.path.event_kind,
                timestamp_ms: current_time_ms(),
            },
        )?;
    }

    Ok(())
}

pub(crate) async fn insert_body_revision_tx(
    transaction: &tokio_postgres::Transaction<'_>,
    workspace_id: &str,
    document_id: &str,
    workspace_cursor: u64,
    actor_id: &str,
    base_text: &str,
    body_text: &str,
    conflicted: bool,
) -> Result<(), StoreError> {
    transaction
        .execute(
            "insert into document_body_revisions \
             (workspace_id, document_id, workspace_cursor, actor_id, base_text, body_text, conflicted) \
             values ($1, $2, $3, $4, $5, $6, $7)",
            &[
                &workspace_id,
                &document_id,
                &(workspace_cursor as i64),
                &actor_id,
                &base_text,
                &body_text,
                &conflicted,
            ],
        )
        .await?;
    Ok(())
}

pub(crate) async fn insert_path_revision_tx(
    transaction: &tokio_postgres::Transaction<'_>,
    workspace_id: &str,
    document_id: &str,
    workspace_cursor: u64,
    actor_id: &str,
    mount_path: &str,
    relative_path: &str,
    deleted: bool,
    event_kind: &str,
) -> Result<(), StoreError> {
    transaction
        .execute(
            "insert into document_path_history \
             (workspace_id, document_id, workspace_cursor, actor_id, mount_path, relative_path, deleted, event_kind) \
             values ($1, $2, $3, $4, $5, $6, $7, $8)",
            &[
                &workspace_id,
                &document_id,
                &(workspace_cursor as i64),
                &actor_id,
                &mount_path,
                &relative_path,
                &deleted,
                &event_kind,
            ],
        )
        .await?;
    Ok(())
}

pub(crate) async fn postgres_list_body_revisions(
    client: &Client,
    workspace_id: &str,
    document_id: &str,
    limit: usize,
) -> Result<Vec<DocumentBodyRevision>, StoreError> {
    let rows = client
        .query(
            "select \
                seq, actor_id, document_id, base_text, body_text, conflicted, \
                (extract(epoch from created_at) * 1000)::bigint as timestamp_ms \
             from document_body_revisions \
             where workspace_id = $1 and document_id = $2 \
             order by seq desc \
             limit $3",
            &[&workspace_id, &document_id, &(limit as i64)],
        )
        .await?;
    let mut revisions = rows
        .into_iter()
        .map(|row| DocumentBodyRevision {
            seq: row.get::<_, i64>("seq") as u64,
            actor_id: row.get::<_, String>("actor_id"),
            document_id: row.get::<_, String>("document_id"),
            base_text: row.get::<_, String>("base_text"),
            body_text: row.get::<_, String>("body_text"),
            conflicted: row.get::<_, bool>("conflicted"),
            timestamp_ms: row.get::<_, i64>("timestamp_ms") as u128,
        })
        .collect::<Vec<_>>();
    revisions.reverse();
    Ok(revisions)
}

pub(crate) async fn postgres_list_path_revisions(
    client: &Client,
    workspace_id: &str,
    document_id: &str,
    limit: usize,
) -> Result<Vec<DocumentPathRevision>, StoreError> {
    let rows = client
        .query(
            "select \
                seq, actor_id, document_id, mount_path, relative_path, deleted, event_kind, \
                (extract(epoch from created_at) * 1000)::bigint as timestamp_ms \
             from document_path_history \
             where workspace_id = $1 and document_id = $2 \
             order by seq desc \
             limit $3",
            &[&workspace_id, &document_id, &(limit as i64)],
        )
        .await?;
    let mut revisions = rows
        .into_iter()
        .map(|row| DocumentPathRevision {
            seq: row.get::<_, i64>("seq") as u64,
            actor_id: row.get::<_, String>("actor_id"),
            document_id: row.get::<_, String>("document_id"),
            mount_path: row.get::<_, String>("mount_path"),
            relative_path: row.get::<_, String>("relative_path"),
            deleted: row.get::<_, bool>("deleted"),
            event_kind: row.get::<_, String>("event_kind"),
            timestamp_ms: row.get::<_, i64>("timestamp_ms") as u128,
        })
        .collect::<Vec<_>>();
    revisions.reverse();
    Ok(revisions)
}

pub(crate) async fn postgres_resolve_document_by_historical_path(
    client: &Client,
    workspace_id: &str,
    mount_path: &str,
    relative_path: &str,
) -> Result<DocumentId, StoreError> {
    let row = client
        .query_opt(
            "select document_id from document_path_history \
             where workspace_id = $1 and mount_path = $2 and relative_path = $3 \
             order by seq desc \
             limit 1",
            &[&workspace_id, &mount_path, &relative_path],
        )
        .await?;
    row.map(|row| DocumentId::new(row.get::<_, String>("document_id")))
        .ok_or_else(|| {
            StoreError::new(format!(
                "no document path history found at {mount_path}/{relative_path}"
            ))
        })
}

pub(crate) async fn postgres_reconstruct_workspace_at_cursor(
    client: &Client,
    workspace_id: &str,
    cursor: u64,
) -> Result<BootstrapSnapshot, StoreError> {
    let path_rows = client
        .query(
            "select distinct on (document_id) \
                document_id, mount_path, relative_path, deleted \
             from document_path_history \
             where workspace_id = $1 and workspace_cursor <= $2 \
             order by document_id, workspace_cursor desc, seq desc",
            &[&workspace_id, &(cursor as i64)],
        )
        .await?;
    let body_rows = client
        .query(
            "select distinct on (document_id) \
                document_id, body_text \
             from document_body_revisions \
             where workspace_id = $1 and workspace_cursor <= $2 \
             order by document_id, workspace_cursor desc, seq desc",
            &[&workspace_id, &(cursor as i64)],
        )
        .await?;

    let body_map = body_rows
        .into_iter()
        .map(|row| {
            (
                row.get::<_, String>("document_id"),
                row.get::<_, String>("body_text"),
            )
        })
        .collect::<HashMap<_, _>>();

    let mut entries = path_rows
        .into_iter()
        .map(|row| ManifestEntry {
            document_id: DocumentId::new(row.get::<_, String>("document_id")),
            mount_relative_path: row.get::<_, String>("mount_path").into(),
            relative_path: row.get::<_, String>("relative_path").into(),
            kind: DocumentKind::Text,
            deleted: row.get::<_, bool>("deleted"),
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        left.mount_relative_path
            .cmp(&right.mount_relative_path)
            .then_with(|| left.relative_path.cmp(&right.relative_path))
            .then_with(|| left.document_id.as_str().cmp(right.document_id.as_str()))
    });

    let mut bodies = entries
        .iter()
        .filter(|entry| !entry.deleted)
        .filter_map(|entry| {
            body_map
                .get(entry.document_id.as_str())
                .map(|text| DocumentBody {
                    document_id: entry.document_id.clone(),
                    text: text.clone(),
                })
        })
        .collect::<Vec<_>>();
    bodies.sort_by(|left, right| left.document_id.as_str().cmp(right.document_id.as_str()));

    Ok(BootstrapSnapshot {
        manifest: ManifestState { entries },
        bodies,
    })
}

pub(crate) async fn postgres_restore_workspace_at_cursor(
    transaction: &tokio_postgres::Transaction<'_>,
    request: &RestoreWorkspaceRequest,
) -> Result<(), StoreError> {
    let current_cursor = current_workspace_cursor_tx(transaction, &request.workspace_id).await?;
    let provided_cursor = request
        .based_on_cursor
        .ok_or_else(|| StoreError::new("workspace restore missing based_on_cursor precondition"))?;
    if provided_cursor != current_cursor {
        return Err(StoreError::conflict(
            "stale_cursor",
            format!(
                "workspace restore based on stale cursor {provided_cursor}; current workspace cursor is {current_cursor}"
            ),
        ));
    }
    if request.cursor > current_cursor {
        return Err(StoreError::new(format!(
            "workspace restore target cursor {} is newer than current workspace cursor {}",
            request.cursor, current_cursor
        )));
    }

    let current_snapshot =
        postgres_current_workspace_snapshot(transaction, &request.workspace_id).await?;
    let target_snapshot = postgres_reconstruct_workspace_at_cursor(
        transaction.client(),
        &request.workspace_id,
        request.cursor,
    )
    .await?;
    let restored_snapshot =
        build_restored_live_workspace_snapshot(&current_snapshot, &target_snapshot);
    let changes =
        diff_workspace_restore_changes(&current_snapshot, &restored_snapshot, request.cursor);

    transaction
        .execute(
            "update document_paths set deleted = true, updated_at = now() where workspace_id = $1",
            &[&request.workspace_id],
        )
        .await?;

    for entry in &restored_snapshot.manifest.entries {
        let existing = transaction
            .query_opt(
                "select 1 from document_paths where workspace_id = $1 and document_id = $2",
                &[&request.workspace_id, &entry.document_id.as_str()],
            )
            .await?;
        if existing.is_some() {
            transaction
                .execute(
                    "update document_paths set mount_path = $3, relative_path = $4, deleted = $5, updated_at = now() \
                     where workspace_id = $1 and document_id = $2",
                    &[
                        &request.workspace_id,
                        &entry.document_id.as_str(),
                        &entry.mount_relative_path.display().to_string(),
                        &entry.relative_path.display().to_string(),
                        &entry.deleted,
                    ],
                )
                .await?;
        } else {
            transaction
                .execute(
                    "insert into document_paths \
                     (document_id, workspace_id, mount_path, relative_path, deleted, manifest_version) \
                     values ($1, $2, $3, $4, $5, 1)",
                    &[
                        &entry.document_id.as_str(),
                        &request.workspace_id,
                        &entry.mount_relative_path.display().to_string(),
                        &entry.relative_path.display().to_string(),
                        &entry.deleted,
                    ],
                )
                .await?;
        }
    }

    for body in &restored_snapshot.bodies {
        transaction
            .execute(
                "insert into document_body_snapshots \
                 (document_id, workspace_id, body_text, compacted_through_seq) \
                 values ($1, $2, $3, 0) \
                 on conflict (document_id) do update set body_text = excluded.body_text, updated_at = now()",
                &[&body.document_id.as_str(), &request.workspace_id, &body.text],
            )
            .await?;
    }

    for change in changes {
        let event_cursor = insert_event_tx(
            transaction,
            &request.workspace_id,
            &request.actor_id,
            Some(change.document_id.as_str()),
            Some(&change.path.mount_path),
            Some(&change.path.relative_path),
            change.kind.clone(),
            &change.summary,
        )
        .await?;
        if let Some(body) = change.body {
            insert_body_revision_tx(
                transaction,
                &request.workspace_id,
                change.document_id.as_str(),
                event_cursor,
                &request.actor_id,
                &body.base_text,
                &body.body_text,
                false,
            )
            .await?;
        }
        insert_path_revision_tx(
            transaction,
            &request.workspace_id,
            change.document_id.as_str(),
            event_cursor,
            &request.actor_id,
            &change.path.mount_path,
            &change.path.relative_path,
            change.path.deleted,
            &change.path.event_kind,
        )
        .await?;
    }

    Ok(())
}

async fn postgres_current_workspace_snapshot(
    transaction: &tokio_postgres::Transaction<'_>,
    workspace_id: &str,
) -> Result<BootstrapSnapshot, StoreError> {
    let rows = transaction
        .query(
            "select \
                dp.document_id, \
                dp.mount_path, \
                dp.relative_path, \
                d.kind, \
                dp.deleted, \
                coalesce(dbs.body_text, '') as body_text \
             from document_paths dp \
             join documents d on d.id = dp.document_id \
             left join document_body_snapshots dbs on dbs.document_id = dp.document_id \
             where dp.workspace_id = $1 \
             order by dp.mount_path, dp.relative_path",
            &[&workspace_id],
        )
        .await?;
    let mut snapshot = BootstrapSnapshot {
        manifest: ManifestState {
            entries: Vec::new(),
        },
        bodies: Vec::new(),
    };
    for row in rows {
        let document_id = DocumentId::new(row.get::<_, String>("document_id"));
        let deleted = row.get::<_, bool>("deleted");
        snapshot.manifest.entries.push(ManifestEntry {
            document_id: document_id.clone(),
            mount_relative_path: row.get::<_, String>("mount_path").into(),
            relative_path: row.get::<_, String>("relative_path").into(),
            kind: DocumentKind::Text,
            deleted,
        });
        if !deleted {
            snapshot.bodies.push(DocumentBody {
                document_id,
                text: row.get::<_, String>("body_text"),
            });
        }
    }
    Ok(snapshot)
}

#[derive(Clone)]
struct WorkspaceRestoreChange {
    document_id: DocumentId,
    kind: ProvenanceEventKind,
    summary: String,
    path: WorkspaceRestorePathChange,
    body: Option<WorkspaceRestoreBodyChange>,
}

#[derive(Clone)]
struct WorkspaceRestorePathChange {
    mount_path: String,
    relative_path: String,
    deleted: bool,
    event_kind: String,
}

#[derive(Clone)]
struct WorkspaceRestoreBodyChange {
    base_text: String,
    body_text: String,
}

fn build_restored_live_workspace_snapshot(
    current: &BootstrapSnapshot,
    target: &BootstrapSnapshot,
) -> BootstrapSnapshot {
    let current_entries = current
        .manifest
        .entries
        .iter()
        .cloned()
        .map(|entry| (entry.document_id.as_str().to_owned(), entry))
        .collect::<HashMap<_, _>>();
    let target_entries = target
        .manifest
        .entries
        .iter()
        .cloned()
        .map(|entry| (entry.document_id.as_str().to_owned(), entry))
        .collect::<HashMap<_, _>>();

    let mut entries = current_entries.clone();
    for (document_id, target_entry) in &target_entries {
        if !target_entry.deleted {
            entries.insert(document_id.clone(), target_entry.clone());
            continue;
        }
        if let Some(current_entry) = entries.get_mut(document_id) {
            current_entry.deleted = true;
        } else {
            entries.insert(document_id.clone(), target_entry.clone());
        }
    }
    for (document_id, current_entry) in &current_entries {
        if !target_entries.contains_key(document_id) && !current_entry.deleted {
            let mut deleted_entry = current_entry.clone();
            deleted_entry.deleted = true;
            entries.insert(document_id.clone(), deleted_entry);
        }
    }

    let mut manifest_entries = entries.into_values().collect::<Vec<_>>();
    manifest_entries.sort_by(|left, right| {
        left.mount_relative_path
            .cmp(&right.mount_relative_path)
            .then_with(|| left.relative_path.cmp(&right.relative_path))
            .then_with(|| left.document_id.as_str().cmp(right.document_id.as_str()))
    });

    let mut bodies = target.bodies.clone();
    bodies.sort_by(|left, right| left.document_id.as_str().cmp(right.document_id.as_str()));

    BootstrapSnapshot {
        manifest: ManifestState {
            entries: manifest_entries,
        },
        bodies,
    }
}

fn diff_workspace_restore_changes(
    current: &BootstrapSnapshot,
    restored: &BootstrapSnapshot,
    target_cursor: u64,
) -> Vec<WorkspaceRestoreChange> {
    let current_entries = current
        .manifest
        .entries
        .iter()
        .map(|entry| (entry.document_id.as_str().to_owned(), entry))
        .collect::<HashMap<_, _>>();
    let restored_entries = restored
        .manifest
        .entries
        .iter()
        .map(|entry| (entry.document_id.as_str().to_owned(), entry))
        .collect::<HashMap<_, _>>();
    let current_bodies = current
        .bodies
        .iter()
        .map(|body| (body.document_id.as_str().to_owned(), body.text.as_str()))
        .collect::<HashMap<_, _>>();
    let restored_bodies = restored
        .bodies
        .iter()
        .map(|body| (body.document_id.as_str().to_owned(), body.text.as_str()))
        .collect::<HashMap<_, _>>();

    let mut document_ids = current_entries
        .keys()
        .chain(restored_entries.keys())
        .cloned()
        .collect::<Vec<_>>();
    document_ids.sort();
    document_ids.dedup();

    let mut changes = Vec::new();
    for document_id in document_ids {
        let Some(restored_entry) = restored_entries.get(&document_id) else {
            continue;
        };
        let current_entry = current_entries.get(&document_id).copied();
        let current_body = current_bodies
            .get(&document_id)
            .copied()
            .unwrap_or_default();
        let restored_body = restored_bodies
            .get(&document_id)
            .copied()
            .unwrap_or_default();

        let current_live = current_entry.map(|entry| !entry.deleted).unwrap_or(false);
        let restored_live = !restored_entry.deleted;
        let path_changed = current_entry
            .map(|entry| {
                entry.mount_relative_path != restored_entry.mount_relative_path
                    || entry.relative_path != restored_entry.relative_path
            })
            .unwrap_or(restored_live);
        let body_changed = current_body != restored_body;

        let Some((kind, summary, path_event_kind)) = restore_change_metadata(
            current_entry,
            restored_entry,
            current_live,
            restored_live,
            path_changed,
            body_changed,
            target_cursor,
        ) else {
            continue;
        };

        let body = if restored_live && body_changed {
            Some(WorkspaceRestoreBodyChange {
                base_text: current_body.to_owned(),
                body_text: restored_body.to_owned(),
            })
        } else {
            None
        };

        changes.push(WorkspaceRestoreChange {
            document_id: restored_entry.document_id.clone(),
            kind,
            summary,
            path: WorkspaceRestorePathChange {
                mount_path: restored_entry.mount_relative_path.display().to_string(),
                relative_path: restored_entry.relative_path.display().to_string(),
                deleted: restored_entry.deleted,
                event_kind: path_event_kind,
            },
            body,
        });
    }

    changes
}

fn restore_change_metadata(
    current_entry: Option<&ManifestEntry>,
    restored_entry: &ManifestEntry,
    current_live: bool,
    restored_live: bool,
    path_changed: bool,
    body_changed: bool,
    target_cursor: u64,
) -> Option<(ProvenanceEventKind, String, String)> {
    let path_display = format!(
        "{}/{}",
        restored_entry.mount_relative_path.display(),
        restored_entry.relative_path.display()
    );
    if current_live && !restored_live {
        return Some((
            ProvenanceEventKind::DocumentDeleted,
            format!(
                "workspace restore to cursor {target_cursor} removed text document from live workspace at {path_display}"
            ),
            "document_deleted".to_owned(),
        ));
    }
    if !current_live && restored_live {
        return Some((
            ProvenanceEventKind::DocumentCreated,
            format!(
                "workspace restore to cursor {target_cursor} restored text document at {path_display}"
            ),
            "workspace_restored".to_owned(),
        ));
    }
    if current_live && restored_live && path_changed {
        return Some((
            ProvenanceEventKind::DocumentMoved,
            format!(
                "workspace restore to cursor {target_cursor} moved text document to {path_display}"
            ),
            "workspace_restored".to_owned(),
        ));
    }
    if current_live && restored_live && body_changed {
        return Some((
            ProvenanceEventKind::DocumentUpdated,
            format!(
                "workspace restore to cursor {target_cursor} restored text document body at {path_display}"
            ),
            "workspace_restored".to_owned(),
        ));
    }
    let _ = current_entry;
    None
}

fn current_time_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time before unix epoch")
        .as_millis()
}
