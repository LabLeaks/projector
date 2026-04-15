/**
@module PROJECTOR.SERVER.POSTGRES_WORKSPACE_BOOTSTRAP
Owns Postgres-backed workspace bootstrap and changes-since reads over workspace rows, mounts, provenance, and reconstructed snapshots.
*/
// @fileimplements PROJECTOR.SERVER.POSTGRES_WORKSPACE_BOOTSTRAP
use std::collections::HashSet;
use std::path::PathBuf;

use projector_domain::{BootstrapSnapshot, DocumentId, SyncEntryKind};

use crate::storage::StoreError;
use crate::storage::bodies::{parse_document_kind, snapshot_from_rows};
use crate::storage::provenance::current_workspace_cursor_tx;

use super::super::normalize_mounts;

pub(crate) async fn postgres_bootstrap_workspace(
    transaction: &tokio_postgres::Transaction<'_>,
    workspace_id: &str,
    mounts: &[PathBuf],
    source_repo_name: Option<&str>,
    sync_entry_kind: Option<SyncEntryKind>,
) -> Result<(BootstrapSnapshot, u64), StoreError> {
    let requested_mounts = normalize_mounts(mounts);

    transaction
        .execute(
            "insert into workspaces (id, owner_actor_id, source_repo_name, entry_kind) values ($1, $2, $3, $4) \
             on conflict (id) do update set \
               source_repo_name = coalesce(workspaces.source_repo_name, excluded.source_repo_name), \
               entry_kind = coalesce(workspaces.entry_kind, excluded.entry_kind)",
            &[
                &workspace_id,
                &"local-owner",
                &source_repo_name,
                &sync_entry_kind
                    .as_ref()
                    .map(format_sync_entry_kind)
                    .map(str::to_owned),
            ],
        )
        .await?;

    let existing_mounts = transaction
        .query(
            "select mount_path from workspace_mounts where workspace_id = $1 order by mount_path",
            &[&workspace_id],
        )
        .await?
        .into_iter()
        .map(|row| row.get::<_, String>(0))
        .collect::<Vec<_>>();

    if existing_mounts.is_empty() {
        for mount in &requested_mounts {
            transaction
                .execute(
                    "insert into workspace_mounts (workspace_id, mount_path) values ($1, $2)",
                    &[&workspace_id, mount],
                )
                .await?;
        }
    } else if existing_mounts != requested_mounts {
        return Err(StoreError::new(format!(
            "workspace {workspace_id} already bound to different mounts"
        )));
    }

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

    let cursor = current_workspace_cursor_tx(transaction, workspace_id).await?;
    let snapshot = snapshot_from_rows(rows, parse_document_kind)?;
    Ok((snapshot, cursor))
}

pub(crate) async fn postgres_changes_since(
    transaction: &tokio_postgres::Transaction<'_>,
    workspace_id: &str,
    since_cursor: u64,
) -> Result<(BootstrapSnapshot, u64), StoreError> {
    let cursor = current_workspace_cursor_tx(transaction, workspace_id).await?;
    if cursor <= since_cursor {
        return Ok((BootstrapSnapshot::default(), cursor));
    }

    let document_ids = transaction
        .query(
            "select distinct document_id \
             from provenance_events \
             where workspace_id = $1 and seq > $2 and document_id is not null",
            &[&workspace_id, &(since_cursor as i64)],
        )
        .await?
        .into_iter()
        .filter_map(|row| row.get::<_, Option<String>>("document_id"))
        .map(DocumentId::new)
        .collect::<HashSet<_>>();

    let rows = if document_ids.is_empty() {
        Vec::new()
    } else {
        let ids = document_ids
            .iter()
            .map(|id| id.as_str().to_owned())
            .collect::<Vec<_>>();
        transaction
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
                 where dp.workspace_id = $1 and dp.document_id = any($2) \
                 order by dp.mount_path, dp.relative_path",
                &[&workspace_id, &ids],
            )
            .await?
    };

    Ok((snapshot_from_rows(rows, parse_document_kind)?, cursor))
}

fn format_sync_entry_kind(kind: &SyncEntryKind) -> &'static str {
    match kind {
        SyncEntryKind::File => "file",
        SyncEntryKind::Directory => "directory",
    }
}
