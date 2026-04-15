/**
@module PROJECTOR.SERVER.SQLITE_WORKSPACE_RESTORE_CLASSIFICATION
Owns classification of per-document workspace-restore changes into created, deleted, moved, or body-restored outcomes.
*/
// @fileimplements PROJECTOR.SERVER.SQLITE_WORKSPACE_RESTORE_CLASSIFICATION
use projector_domain::ProvenanceEventKind;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RestoreChangeClass {
    Deleted,
    RestoredLive,
    Moved,
    BodyRestored,
}

impl RestoreChangeClass {
    pub(super) fn provenance_kind(self) -> ProvenanceEventKind {
        match self {
            RestoreChangeClass::Deleted => ProvenanceEventKind::DocumentDeleted,
            RestoreChangeClass::RestoredLive => ProvenanceEventKind::DocumentCreated,
            RestoreChangeClass::Moved => ProvenanceEventKind::DocumentMoved,
            RestoreChangeClass::BodyRestored => ProvenanceEventKind::DocumentUpdated,
        }
    }

    pub(super) fn summary_code(self) -> &'static str {
        match self {
            RestoreChangeClass::Deleted => "document_deleted",
            RestoreChangeClass::RestoredLive
            | RestoreChangeClass::Moved
            | RestoreChangeClass::BodyRestored => "workspace_restored",
        }
    }
}

pub(super) fn classify_restore_change(
    current_live: bool,
    restored_live: bool,
    path_changed: bool,
    body_changed: bool,
) -> Option<RestoreChangeClass> {
    match (current_live, restored_live, path_changed, body_changed) {
        (true, false, _, _) => Some(RestoreChangeClass::Deleted),
        (false, true, _, _) => Some(RestoreChangeClass::RestoredLive),
        (true, true, true, _) => Some(RestoreChangeClass::Moved),
        (true, true, false, true) => Some(RestoreChangeClass::BodyRestored),
        _ => None,
    }
}
