/**
@module PROJECTOR.SERVER.SQLITE_WORKSPACE_RESTORE_PLAN
Owns the SQLite workspace-restore planning seam that turns current and reconstructed snapshots into append-only restore changes.
*/
// @fileimplements PROJECTOR.SERVER.SQLITE_WORKSPACE_RESTORE_PLAN
pub(super) mod diff;
pub(super) mod metadata;
