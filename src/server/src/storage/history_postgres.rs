/**
@module PROJECTOR.SERVER.POSTGRES_HISTORY
Owns Postgres-backed retained-history storage, preview, compaction-policy persistence, and workspace-history reconstruction and restore.
*/
// @fileimplements PROJECTOR.SERVER.POSTGRES_HISTORY
use std::collections::HashMap;
use std::path::PathBuf;

use projector_domain::{
    BootstrapSnapshot, ClearHistoryCompactionPolicyRequest, DocumentBodyPurgeMatch,
    DocumentBodyRedactionMatch, DocumentBodyRevision, DocumentId, DocumentKind,
    DocumentPathRevision, GetHistoryCompactionPolicyResponse, HistoryCompactionPolicy,
    ManifestEntry, PreviewPurgeDocumentBodyHistoryRequest, PreviewRedactDocumentBodyHistoryRequest,
    ProvenanceEventKind, PurgeDocumentBodyHistoryRequest, RedactDocumentBodyHistoryRequest,
    RestoreWorkspaceRequest, SetHistoryCompactionPolicyRequest,
};
use tokio_postgres::{Client, GenericClient};

use super::StoreError;
use super::body_persistence::{AsyncBodyPersistence, PostgresBodyPersistence};
use super::body_projection::{snapshot_from_current_rows, snapshot_from_manifest_entries};
use super::body_state::{BodyStateModel, FULL_TEXT_BODY_MODEL, RetainedBodyHistoryKind};
use super::history::{FileBodyRevision, insert_path_revision_tx, parse_public_path_event_kind};
use super::history_compaction::{
    StoredHistoryCompactionPolicyOverride, compact_document_body_revisions,
    history_compaction_response, normalize_history_compaction_path, replay_body_revision_run,
    resolve_history_compaction_policy, validate_history_compaction_policy,
};
use super::history_restore::{
    build_restored_live_workspace_snapshot, diff_workspace_restore_changes,
};
use super::history_surgery::{
    ensure_expected_history_match_set, purge_history_summary, redact_history_summary,
    retained_purge_matches, retained_redaction_matches,
};
use super::provenance::insert_event_tx;

async fn postgres_read_history_compaction_policies(
    client: &impl GenericClient,
    workspace_id: &str,
) -> Result<Vec<StoredHistoryCompactionPolicyOverride>, StoreError> {
    let rows = client
        .query(
            "select repo_relative_path, revisions, frequency \
             from history_compaction_policies \
             where workspace_id = $1 \
             order by repo_relative_path asc",
            &[&workspace_id],
        )
        .await?;
    rows.into_iter()
        .map(|row| {
            let revisions = u32::try_from(row.get::<_, i32>("revisions")).map_err(|_| {
                StoreError::new("history compaction revisions must be non-negative")
            })?;
            let frequency = u32::try_from(row.get::<_, i32>("frequency")).map_err(|_| {
                StoreError::new("history compaction frequency must be non-negative")
            })?;
            if revisions == 0 {
                return Err(StoreError::new(
                    "history compaction revisions must be at least 1",
                ));
            }
            if frequency == 0 {
                return Err(StoreError::new(
                    "history compaction frequency must be at least 1",
                ));
            }
            Ok(StoredHistoryCompactionPolicyOverride {
                repo_relative_path: PathBuf::from(row.get::<_, String>("repo_relative_path")),
                policy: HistoryCompactionPolicy {
                    revisions,
                    frequency,
                },
            })
        })
        .collect()
}

pub(crate) async fn postgres_get_history_compaction_policy(
    client: &Client,
    workspace_id: &str,
    repo_relative_path: &str,
) -> Result<GetHistoryCompactionPolicyResponse, StoreError> {
    let normalized_path = normalize_history_compaction_path(repo_relative_path)?;
    Ok(history_compaction_response(
        &postgres_read_history_compaction_policies(client, workspace_id).await?,
        &normalized_path,
    ))
}

pub(crate) async fn postgres_set_history_compaction_policy(
    transaction: &tokio_postgres::Transaction<'_>,
    request: &SetHistoryCompactionPolicyRequest,
) -> Result<(), StoreError> {
    validate_history_compaction_policy(&request.policy)?;
    let normalized_path = normalize_history_compaction_path(&request.repo_relative_path)?;
    let revisions = i32::try_from(request.policy.revisions)
        .map_err(|_| StoreError::new("history compaction revisions exceeded postgres range"))?;
    let frequency = i32::try_from(request.policy.frequency)
        .map_err(|_| StoreError::new("history compaction frequency exceeded postgres range"))?;
    transaction
        .execute(
            "insert into history_compaction_policies (workspace_id, repo_relative_path, revisions, frequency) \
             values ($1, $2, $3, $4) \
             on conflict (workspace_id, repo_relative_path) do update set \
               revisions = excluded.revisions, \
               frequency = excluded.frequency",
            &[
                &request.workspace_id,
                &normalized_path.display().to_string(),
                &revisions,
                &frequency,
            ],
        )
        .await?;
    Ok(())
}

pub(crate) async fn postgres_clear_history_compaction_policy(
    transaction: &tokio_postgres::Transaction<'_>,
    request: &ClearHistoryCompactionPolicyRequest,
) -> Result<bool, StoreError> {
    let normalized_path = normalize_history_compaction_path(&request.repo_relative_path)?;
    let removed = transaction
        .execute(
            "delete from history_compaction_policies \
             where workspace_id = $1 and repo_relative_path = $2",
            &[
                &request.workspace_id,
                &normalized_path.display().to_string(),
            ],
        )
        .await?;
    Ok(removed > 0)
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
                seq, actor_id, document_id, checkpoint_anchor_seq, history_kind, base_text, body_text, conflicted, \
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
        .map(|row| {
            let kind =
                RetainedBodyHistoryKind::parse(row.get::<_, String>("history_kind").as_str())
                    .map_err(StoreError::new)?;
            Ok(FileBodyRevision {
                seq: row.get::<_, i64>("seq") as u64,
                workspace_cursor: 0,
                actor_id: row.get::<_, String>("actor_id"),
                document_id: row.get::<_, String>("document_id"),
                checkpoint_anchor_seq: row
                    .get::<_, Option<i64>>("checkpoint_anchor_seq")
                    .map(|seq| seq as u64),
                history_kind: kind,
                base_text: row.get::<_, String>("base_text"),
                body_text: row.get::<_, String>("body_text"),
                conflicted: row.get::<_, bool>("conflicted"),
                timestamp_ms: row.get::<_, i64>("timestamp_ms") as u128,
            }
            .to_public_revision())
        })
        .collect::<Result<Vec<_>, StoreError>>()?;
    revisions.reverse();
    Ok(revisions)
}

pub(crate) async fn postgres_preview_redact_document_body_history(
    client: &Client,
    request: &PreviewRedactDocumentBodyHistoryRequest,
) -> Result<Vec<DocumentBodyRedactionMatch>, StoreError> {
    let rows = client
        .query(
            "select seq, actor_id, document_id, checkpoint_anchor_seq, history_kind, base_text, body_text, conflicted, \
                (extract(epoch from created_at) * 1000)::bigint as timestamp_ms \
             from document_body_revisions \
             where workspace_id = $1 and document_id = $2 \
             order by seq asc",
            &[&request.workspace_id, &request.document_id],
        )
        .await?;
    let revisions = rows
        .into_iter()
        .map(|row| {
            let kind =
                RetainedBodyHistoryKind::parse(row.get::<_, String>("history_kind").as_str())
                    .map_err(StoreError::new)?;
            Ok(FileBodyRevision {
                seq: row.get::<_, i64>("seq") as u64,
                workspace_cursor: 0,
                actor_id: row.get::<_, String>("actor_id"),
                document_id: row.get::<_, String>("document_id"),
                checkpoint_anchor_seq: row
                    .get::<_, Option<i64>>("checkpoint_anchor_seq")
                    .map(|seq| seq as u64),
                history_kind: kind,
                base_text: row.get::<_, String>("base_text"),
                body_text: row.get::<_, String>("body_text"),
                conflicted: row.get::<_, bool>("conflicted"),
                timestamp_ms: row.get::<_, i64>("timestamp_ms") as u128,
            })
        })
        .collect::<Result<Vec<_>, StoreError>>()?;
    retained_redaction_matches(
        revisions,
        &request.document_id,
        &request.exact_text,
        request.limit,
    )
}

pub(crate) async fn postgres_preview_purge_document_body_history(
    client: &Client,
    request: &PreviewPurgeDocumentBodyHistoryRequest,
) -> Result<Vec<DocumentBodyPurgeMatch>, StoreError> {
    let rows = client
        .query(
            "select seq, actor_id, document_id, checkpoint_anchor_seq, history_kind, base_text, body_text, conflicted, \
                (extract(epoch from created_at) * 1000)::bigint as timestamp_ms \
             from document_body_revisions \
             where workspace_id = $1 and document_id = $2 \
             order by seq asc",
            &[&request.workspace_id, &request.document_id],
        )
        .await?;
    let revisions = rows
        .into_iter()
        .map(|row| {
            let kind =
                RetainedBodyHistoryKind::parse(row.get::<_, String>("history_kind").as_str())
                    .map_err(StoreError::new)?;
            Ok(FileBodyRevision {
                seq: row.get::<_, i64>("seq") as u64,
                workspace_cursor: 0,
                actor_id: row.get::<_, String>("actor_id"),
                document_id: row.get::<_, String>("document_id"),
                checkpoint_anchor_seq: row
                    .get::<_, Option<i64>>("checkpoint_anchor_seq")
                    .map(|seq| seq as u64),
                history_kind: kind,
                base_text: row.get::<_, String>("base_text"),
                body_text: row.get::<_, String>("body_text"),
                conflicted: row.get::<_, bool>("conflicted"),
                timestamp_ms: row.get::<_, i64>("timestamp_ms") as u128,
            })
        })
        .collect::<Result<Vec<_>, StoreError>>()?;
    Ok(retained_purge_matches(
        revisions,
        &request.document_id,
        request.limit,
    ))
}

pub(crate) async fn postgres_enforce_history_compaction_policy(
    transaction: &tokio_postgres::Transaction<'_>,
    workspace_id: &str,
    document_id: &str,
) -> Result<(), StoreError> {
    let Some(path_row) = transaction
        .query_opt(
            "select mount_path, relative_path from document_paths \
             where workspace_id = $1 and document_id = $2 and deleted = false",
            &[&workspace_id, &document_id],
        )
        .await?
    else {
        return Ok(());
    };
    let repo_relative_path = PathBuf::from(path_row.get::<_, String>("mount_path"))
        .join(path_row.get::<_, String>("relative_path"));
    let resolved = resolve_history_compaction_policy(
        &postgres_read_history_compaction_policies(transaction, workspace_id).await?,
        &repo_relative_path,
    );
    let rows = transaction
        .query(
            "select seq, actor_id, document_id, checkpoint_anchor_seq, history_kind, base_text, body_text, conflicted, \
                (extract(epoch from created_at) * 1000)::bigint as timestamp_ms \
             from document_body_revisions \
             where workspace_id = $1 and document_id = $2 \
             order by seq asc",
            &[&workspace_id, &document_id],
        )
        .await?;
    let original = rows
        .into_iter()
        .map(|row| {
            let kind =
                RetainedBodyHistoryKind::parse(row.get::<_, String>("history_kind").as_str())
                    .map_err(StoreError::new)?;
            Ok(FileBodyRevision {
                seq: row.get::<_, i64>("seq") as u64,
                workspace_cursor: 0,
                actor_id: row.get::<_, String>("actor_id"),
                document_id: row.get::<_, String>("document_id"),
                checkpoint_anchor_seq: row
                    .get::<_, Option<i64>>("checkpoint_anchor_seq")
                    .map(|seq| seq as u64),
                history_kind: kind,
                base_text: row.get::<_, String>("base_text"),
                body_text: row.get::<_, String>("body_text"),
                conflicted: row.get::<_, bool>("conflicted"),
                timestamp_ms: row.get::<_, i64>("timestamp_ms") as u128,
            })
        })
        .collect::<Result<Vec<_>, StoreError>>()?;
    let compacted = compact_document_body_revisions(&original, document_id, &resolved.policy)?;
    if compacted == original {
        return Ok(());
    }
    let compacted_by_seq = compacted
        .iter()
        .map(|revision| (revision.seq, revision))
        .collect::<HashMap<_, _>>();
    for revision in &compacted {
        transaction
            .execute(
                "update document_body_revisions \
                 set checkpoint_anchor_seq = $3, history_kind = $4, base_text = $5, body_text = $6, conflicted = $7 \
                 where workspace_id = $1 and seq = $2",
                &[
                    &workspace_id,
                    &(revision.seq as i64),
                    &revision.checkpoint_anchor_seq.map(|seq| seq as i64),
                    &revision.history_kind.as_str(),
                    &revision.base_text,
                    &revision.body_text,
                    &revision.conflicted,
                ],
            )
            .await?;
    }
    let dropped = original
        .iter()
        .filter(|revision| !compacted_by_seq.contains_key(&revision.seq))
        .map(|revision| revision.seq as i64)
        .collect::<Vec<_>>();
    if !dropped.is_empty() {
        transaction
            .execute(
                "delete from document_body_revisions \
                 where workspace_id = $1 and document_id = $2 and seq = any($3)",
                &[&workspace_id, &document_id, &dropped],
            )
            .await?;
    }
    Ok(())
}

pub(crate) async fn postgres_purge_document_body_history(
    transaction: &tokio_postgres::Transaction<'_>,
    request: &PurgeDocumentBodyHistoryRequest,
) -> Result<(), StoreError> {
    let matched_rows = transaction
        .query(
            "select seq from document_body_revisions \
             where workspace_id = $1 and document_id = $2 and (base_text <> '' or body_text <> '') \
             order by seq asc",
            &[&request.workspace_id, &request.document_id],
        )
        .await?;
    let matched_seqs = matched_rows
        .into_iter()
        .map(|row| row.get::<_, i64>(0) as u64)
        .collect::<Vec<_>>();
    if matched_seqs.is_empty() {
        return Err(StoreError::new(format!(
            "document {} has no retained body history in workspace {}",
            request.document_id, request.workspace_id
        )));
    }
    ensure_expected_history_match_set(
        &request.document_id,
        request.expected_match_seqs.as_ref(),
        &matched_seqs,
        "purge",
    )?;
    let matched = transaction
        .execute(
            "update document_body_revisions \
             set base_text = '', body_text = '' \
             where workspace_id = $1 and document_id = $2 and (base_text <> '' or body_text <> '')",
            &[&request.workspace_id, &request.document_id],
        )
        .await?;
    if matched == 0 {
        return Err(StoreError::new(format!(
            "document {} has no retained body history in workspace {}",
            request.document_id, request.workspace_id
        )));
    }
    transaction
        .execute(
            "delete from document_body_updates where workspace_id = $1 and document_id = $2",
            &[&request.workspace_id, &request.document_id],
        )
        .await?;
    let live_path = transaction
        .query_opt(
            "select mount_path, relative_path from document_paths \
             where workspace_id = $1 and document_id = $2 and deleted = false",
            &[&request.workspace_id, &request.document_id],
        )
        .await?;
    let mount_relative_path = live_path
        .as_ref()
        .map(|row| row.get::<_, String>("mount_path"));
    let relative_path = live_path
        .as_ref()
        .map(|row| row.get::<_, String>("relative_path"));
    insert_event_tx(
        transaction,
        &request.workspace_id,
        &request.actor_id,
        Some(&request.document_id),
        mount_relative_path.as_deref(),
        relative_path.as_deref(),
        ProvenanceEventKind::DocumentHistoryPurged,
        &purge_history_summary(
            request.document_id.as_str(),
            mount_relative_path.as_deref(),
            relative_path.as_deref(),
        ),
    )
    .await?;
    Ok(())
}

pub(crate) async fn postgres_redact_document_body_history(
    transaction: &tokio_postgres::Transaction<'_>,
    request: &RedactDocumentBodyHistoryRequest,
) -> Result<(), StoreError> {
    let rows = transaction
        .query(
            "select seq, actor_id, document_id, checkpoint_anchor_seq, history_kind, base_text, body_text, conflicted, \
                (extract(epoch from created_at) * 1000)::bigint as timestamp_ms \
             from document_body_revisions \
             where workspace_id = $1 and document_id = $2 \
             order by seq asc",
            &[&request.workspace_id, &request.document_id],
        )
        .await?;
    let revisions = rows
        .into_iter()
        .map(|row| {
            let kind =
                RetainedBodyHistoryKind::parse(row.get::<_, String>("history_kind").as_str())
                    .map_err(StoreError::new)?;
            Ok(FileBodyRevision {
                seq: row.get::<_, i64>("seq") as u64,
                workspace_cursor: 0,
                actor_id: row.get::<_, String>("actor_id"),
                document_id: row.get::<_, String>("document_id"),
                checkpoint_anchor_seq: row
                    .get::<_, Option<i64>>("checkpoint_anchor_seq")
                    .map(|seq| seq as u64),
                history_kind: kind,
                base_text: row.get::<_, String>("base_text"),
                body_text: row.get::<_, String>("body_text"),
                conflicted: row.get::<_, bool>("conflicted"),
                timestamp_ms: row.get::<_, i64>("timestamp_ms") as u128,
            })
        })
        .collect::<Result<Vec<_>, StoreError>>()?;

    let redacted_revisions = revisions
        .into_iter()
        .filter_map(|revision| revision.redacted(&request.exact_text).transpose())
        .collect::<Result<Vec<_>, StoreError>>()?;
    let matched_seqs = redacted_revisions
        .iter()
        .map(|revision| revision.seq)
        .collect::<Vec<_>>();
    if matched_seqs.is_empty() {
        return Err(StoreError::new(format!(
            "document {} has no retained body history matching {:?} in workspace {}",
            request.document_id, request.exact_text, request.workspace_id
        )));
    }
    ensure_expected_history_match_set(
        &request.document_id,
        request.expected_match_seqs.as_ref(),
        &matched_seqs,
        "redaction",
    )?;
    for redacted in &redacted_revisions {
        transaction
            .execute(
                "update document_body_revisions \
                 set history_kind = $3, base_text = $4, body_text = $5, conflicted = $6 \
                 where workspace_id = $1 and seq = $2",
                &[
                    &request.workspace_id,
                    &(redacted.seq as i64),
                    &redacted.history_kind.as_str(),
                    &redacted.base_text,
                    &redacted.body_text,
                    &redacted.conflicted,
                ],
            )
            .await?;
    }
    let live_path = transaction
        .query_opt(
            "select mount_path, relative_path from document_paths \
             where workspace_id = $1 and document_id = $2 and deleted = false",
            &[&request.workspace_id, &request.document_id],
        )
        .await?;
    let mount_relative_path = live_path
        .as_ref()
        .map(|row| row.get::<_, String>("mount_path"));
    let relative_path = live_path
        .as_ref()
        .map(|row| row.get::<_, String>("relative_path"));
    insert_event_tx(
        transaction,
        &request.workspace_id,
        &request.actor_id,
        Some(&request.document_id),
        mount_relative_path.as_deref(),
        relative_path.as_deref(),
        ProvenanceEventKind::DocumentHistoryRedacted,
        &redact_history_summary(
            request.document_id.as_str(),
            mount_relative_path.as_deref(),
            relative_path.as_deref(),
        ),
    )
    .await?;
    Ok(())
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
        .map(|row| {
            Ok(DocumentPathRevision {
                seq: row.get::<_, i64>("seq") as u64,
                actor_id: row.get::<_, String>("actor_id"),
                document_id: row.get::<_, String>("document_id"),
                mount_path: row.get::<_, String>("mount_path"),
                relative_path: row.get::<_, String>("relative_path"),
                deleted: row.get::<_, bool>("deleted"),
                event_kind: parse_public_path_event_kind(&row.get::<_, String>("event_kind"))?,
                timestamp_ms: row.get::<_, i64>("timestamp_ms") as u128,
            })
        })
        .collect::<Result<Vec<_>, StoreError>>()?;
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
            "select \
                seq, workspace_cursor, document_id, checkpoint_anchor_seq, history_kind, base_text, body_text, conflicted \
             from document_body_revisions \
             where workspace_id = $1 and workspace_cursor <= $2 \
             order by workspace_cursor asc, seq asc",
            &[&workspace_id, &(cursor as i64)],
        )
        .await?;

    let body_revisions = body_rows
        .into_iter()
        .map(|row| {
            let kind =
                RetainedBodyHistoryKind::parse(row.get::<_, String>("history_kind").as_str())
                    .map_err(StoreError::new)?;
            Ok(FileBodyRevision {
                seq: row.get::<_, i64>("seq") as u64,
                workspace_cursor: row.get::<_, i64>("workspace_cursor") as u64,
                actor_id: String::new(),
                document_id: row.get::<_, String>("document_id"),
                checkpoint_anchor_seq: row
                    .get::<_, Option<i64>>("checkpoint_anchor_seq")
                    .map(|seq| seq as u64),
                history_kind: kind,
                base_text: row.get::<_, String>("base_text"),
                body_text: row.get::<_, String>("body_text"),
                conflicted: row.get::<_, bool>("conflicted"),
                timestamp_ms: 0,
            })
        })
        .collect::<Result<Vec<_>, StoreError>>()?;
    let body_map = replay_body_revision_run(body_revisions);

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

    Ok(snapshot_from_manifest_entries(entries, |document_id| {
        body_map.get(document_id.as_str()).cloned()
    }))
}

pub(crate) async fn postgres_restore_workspace_at_cursor(
    transaction: &tokio_postgres::Transaction<'_>,
    request: &RestoreWorkspaceRequest,
) -> Result<(), StoreError> {
    let current_cursor =
        super::provenance::current_workspace_cursor_tx(transaction, &request.workspace_id).await?;
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

    let body_persistence = PostgresBodyPersistence::new(transaction, &request.workspace_id);
    for body in &restored_snapshot.bodies {
        let state = FULL_TEXT_BODY_MODEL.state_from_materialized_text(body.text.clone());
        body_persistence
            .write_current_state(body.document_id.as_str(), &state)
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
            body_persistence
                .append_retained_history(
                    event_cursor,
                    &request.actor_id,
                    change.document_id.as_str(),
                    &FULL_TEXT_BODY_MODEL.checkpoint_history(body.base_text, body.body_text),
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
                coalesce(dbs.state_kind, 'full_text_merge_v1') as state_kind, \
                coalesce(dbs.body_text, '') as body_text \
             from document_paths dp \
             join documents d on d.id = dp.document_id \
             left join document_body_snapshots dbs on dbs.document_id = dp.document_id \
             where dp.workspace_id = $1 \
             order by dp.mount_path, dp.relative_path",
            &[&workspace_id],
        )
        .await?;
    snapshot_from_current_rows(rows, |_| Ok(DocumentKind::Text))
}
