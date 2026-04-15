/**
@module PROJECTOR.DOMAIN.MANIFEST
Defines canonical document manifest types: stable ids, mounted paths, document kind, and deleted state.
*/
// @fileimplements PROJECTOR.DOMAIN.MANIFEST
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::DocumentId;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DocumentKind {
    Text,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub document_id: DocumentId,
    pub mount_relative_path: PathBuf,
    pub relative_path: PathBuf,
    pub kind: DocumentKind,
    pub deleted: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ManifestState {
    pub entries: Vec<ManifestEntry>,
}
