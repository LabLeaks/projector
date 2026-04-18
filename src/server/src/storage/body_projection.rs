/**
@module PROJECTOR.SERVER.BODY_PROJECTION
Owns projection of backend-specific body-state reads into live bootstrap snapshot bodies so read paths can evolve beyond plain full-text rows without rewriting reconstruction and bootstrap callsites.
*/
// @fileimplements PROJECTOR.SERVER.BODY_PROJECTION
use std::path::PathBuf;

use projector_domain::{
    BootstrapSnapshot, DocumentBody, DocumentId, DocumentKind, ManifestEntry, ManifestState,
};

use super::StoreError;
use super::body_state::{
    BodyStateModel, CanonicalBodyState, CanonicalBodyStateKind, FULL_TEXT_BODY_MODEL,
};

pub(crate) fn live_body_from_state(
    document_id: DocumentId,
    state: &CanonicalBodyState,
) -> DocumentBody {
    state.clone().into_document_body(document_id)
}

pub(crate) fn snapshot_from_manifest_entries<F>(
    entries: Vec<ManifestEntry>,
    mut state_for_document: F,
) -> BootstrapSnapshot
where
    F: FnMut(&DocumentId) -> Option<CanonicalBodyState>,
{
    let mut bodies = entries
        .iter()
        .filter(|entry| !entry.deleted)
        .filter_map(|entry| {
            state_for_document(&entry.document_id)
                .map(|state| live_body_from_state(entry.document_id.clone(), &state))
        })
        .collect::<Vec<_>>();
    bodies.sort_by(|left, right| left.document_id.as_str().cmp(right.document_id.as_str()));

    BootstrapSnapshot {
        manifest: ManifestState { entries },
        bodies,
    }
}

pub(crate) fn snapshot_from_current_rows<F>(
    rows: Vec<tokio_postgres::Row>,
    parse_kind: F,
) -> Result<BootstrapSnapshot, StoreError>
where
    F: Fn(&str) -> Result<DocumentKind, StoreError>,
{
    let mut entries = Vec::new();
    let mut body_rows = Vec::<(DocumentId, (CanonicalBodyStateKind, String))>::new();

    for row in rows {
        let document_id = DocumentId::new(row.get::<_, String>("document_id"));
        let deleted = row.get::<_, bool>("deleted");
        let kind = parse_kind(&row.get::<_, String>("kind"))?;
        entries.push(ManifestEntry {
            document_id: document_id.clone(),
            mount_relative_path: PathBuf::from(row.get::<_, String>("mount_path")),
            relative_path: PathBuf::from(row.get::<_, String>("relative_path")),
            kind,
            deleted,
        });
        if !deleted {
            body_rows.push((
                document_id,
                (
                    CanonicalBodyStateKind::parse(row.get::<_, String>("state_kind").as_str())
                        .map_err(StoreError::new)?,
                    row.get::<_, String>("body_text"),
                ),
            ));
        }
    }

    let body_map = body_rows.into_iter().collect::<std::collections::HashMap<_, _>>();
    Ok(snapshot_from_manifest_entries(entries, |document_id| {
        body_map
            .get(document_id)
            .map(|(kind, text)| FULL_TEXT_BODY_MODEL.state_from_storage_record(*kind, text.clone()))
    }))
}
