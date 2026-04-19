/**
@module PROJECTOR.SERVER.POSTGRES_SYNC_ENTRY_DISCOVERY
Owns Postgres-backed remote sync-entry listing, kind inference, and preview rendering for `projector get`.
*/
// @fileimplements PROJECTOR.SERVER.POSTGRES_SYNC_ENTRY_DISCOVERY
use projector_domain::SyncEntrySummary;

use crate::storage::StoreError;
use crate::storage::history::postgres_reconstruct_workspace_at_cursor;

use super::super::parse_sync_entry_kind;
use super::super::{infer_sync_entry_kind, sync_entry_preview_summary};

pub(crate) async fn postgres_list_sync_entries(
    client: &tokio_postgres::Client,
    limit: usize,
) -> Result<Vec<SyncEntrySummary>, StoreError> {
    let rows = client
        .query(
            "select w.id, w.source_repo_name, w.entry_kind, wm.mount_path, \
                    (extract(epoch from max(pe.created_at)) * 1000)::bigint as last_updated_ms \
             from workspaces w \
             join workspace_mounts wm on wm.workspace_id = w.id \
             left join provenance_events pe on pe.workspace_id = w.id \
             group by w.id, w.source_repo_name, w.entry_kind, wm.mount_path \
             order by max(pe.created_at) desc nulls last, wm.mount_path asc \
             limit $1",
            &[&(limit as i64)],
        )
        .await?;

    let mut entries = Vec::new();
    for row in rows {
        let workspace_id = row.get::<_, String>("id");
        let source_repo_name = row.get::<_, Option<String>>("source_repo_name");
        let remote_path = row.get::<_, String>("mount_path");
        let snapshot =
            postgres_reconstruct_workspace_at_cursor(client, &workspace_id, i64::MAX as u64)
                .await
                .unwrap_or_default();
        let kind = row
            .get::<_, Option<String>>("entry_kind")
            .as_deref()
            .map(parse_sync_entry_kind)
            .transpose()?
            .unwrap_or_else(|| infer_sync_entry_kind(&snapshot));
        let last_updated_ms = row
            .get::<_, Option<i64>>("last_updated_ms")
            .map(|value| value as u128);
        entries.push(SyncEntrySummary {
            sync_entry_id: workspace_id.clone(),
            workspace_id,
            remote_path,
            kind: kind.clone(),
            source_repo_name,
            last_updated_ms,
            preview: sync_entry_preview_summary(&snapshot, &kind),
        });
    }

    Ok(entries)
}
