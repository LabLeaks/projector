/**
@module PROJECTOR.RUNTIME.MATERIALIZED_BODIES
Persists and loads the repo-local checkpoint of last-materialized live document bodies under `.projector/` so runtime updates can send the actual local base text they diverged from.
*/
// @fileimplements PROJECTOR.RUNTIME.MATERIALIZED_BODIES
use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use projector_domain::{BootstrapSnapshot, DocumentId};

fn materialized_bodies_path(projector_dir: &Path) -> PathBuf {
    projector_dir.join("materialized_bodies.json")
}

pub(crate) fn load_materialized_body_texts(
    projector_dir: &Path,
) -> Result<HashMap<DocumentId, String>, Box<dyn Error>> {
    let path = materialized_bodies_path(projector_dir);
    if !path.exists() {
        return Ok(HashMap::new());
    }

    let raw = fs::read(path)?;
    let stored =
        serde_json::from_slice::<HashMap<String, String>>(&raw).map_err(Box::<dyn Error>::from)?;
    Ok(stored
        .into_iter()
        .map(|(document_id, text)| (DocumentId::new(document_id), text))
        .collect())
}

pub(crate) fn save_materialized_body_texts(
    projector_dir: &Path,
    snapshot: &BootstrapSnapshot,
) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(projector_dir)?;
    let live_document_ids = snapshot
        .manifest
        .entries
        .iter()
        .filter(|entry| !entry.deleted)
        .map(|entry| entry.document_id.clone())
        .collect::<std::collections::HashSet<_>>();
    let stored = snapshot
        .bodies
        .iter()
        .filter(|body| live_document_ids.contains(&body.document_id))
        .map(|body| (body.document_id.as_str().to_owned(), body.text.clone()))
        .collect::<BTreeMap<_, _>>();
    let content = serde_json::to_vec_pretty(&stored).map_err(Box::<dyn Error>::from)?;
    fs::write(materialized_bodies_path(projector_dir), content)?;
    Ok(())
}
