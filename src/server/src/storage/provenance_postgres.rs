/**
@module PROJECTOR.SERVER.POSTGRES_PROVENANCE
Owns Postgres-backed workspace cursor reads, event inserts, and event listing.
*/
// @fileimplements PROJECTOR.SERVER.POSTGRES_PROVENANCE
use projector_domain::{DocumentId, ProvenanceEvent};
use tokio_postgres::Client;

use super::StoreError;
use super::provenance::{event_kind_db_value, parse_event_kind};

pub(crate) async fn current_workspace_cursor_tx(
    transaction: &tokio_postgres::Transaction<'_>,
    workspace_id: &str,
) -> Result<u64, StoreError> {
    Ok(transaction
        .query_one(
            "select coalesce(max(seq), 0)::bigint as cursor from provenance_events where workspace_id = $1",
            &[&workspace_id],
        )
        .await?
        .get::<_, i64>("cursor") as u64)
}

pub(crate) async fn insert_event_tx(
    transaction: &tokio_postgres::Transaction<'_>,
    workspace_id: &str,
    actor_id: &str,
    document_id: Option<&str>,
    mount_path: Option<&str>,
    relative_path: Option<&str>,
    kind: projector_domain::ProvenanceEventKind,
    summary: &str,
) -> Result<u64, StoreError> {
    let kind_value = event_kind_db_value(&kind);
    let row = transaction
        .query_one(
            "insert into provenance_events \
             (workspace_id, actor_id, document_id, mount_path, relative_path, event_kind, summary) \
             values ($1, $2, $3, $4, $5, $6, $7) \
             returning seq",
            &[
                &workspace_id,
                &actor_id,
                &document_id,
                &mount_path,
                &relative_path,
                &kind_value,
                &summary,
            ],
        )
        .await?;
    Ok(row.get::<_, i64>("seq") as u64)
}

pub(crate) async fn postgres_list_events(
    client: &Client,
    workspace_id: &str,
    limit: usize,
) -> Result<Vec<ProvenanceEvent>, StoreError> {
    let rows = client
        .query(
            "select \
                seq, \
                actor_id, \
                document_id, \
                mount_path, \
                relative_path, \
                event_kind, \
                summary, \
                (extract(epoch from created_at) * 1000)::bigint as timestamp_ms \
             from provenance_events \
             where workspace_id = $1 \
             order by seq desc \
             limit $2",
            &[&workspace_id, &(limit as i64)],
        )
        .await?;

    let mut events = rows
        .into_iter()
        .map(|row| {
            Ok(ProvenanceEvent {
                cursor: row.get::<_, i64>("seq") as u64,
                timestamp_ms: row.get::<_, i64>("timestamp_ms") as u128,
                actor_id: projector_domain::ActorId::new(row.get::<_, String>("actor_id")),
                document_id: row
                    .get::<_, Option<String>>("document_id")
                    .map(DocumentId::new),
                mount_relative_path: row.get::<_, Option<String>>("mount_path"),
                relative_path: row.get::<_, Option<String>>("relative_path"),
                summary: row.get::<_, String>("summary"),
                kind: parse_event_kind(&row.get::<_, String>("event_kind"))?,
            })
        })
        .collect::<Result<Vec<_>, StoreError>>()?;
    events.reverse();
    Ok(events)
}
