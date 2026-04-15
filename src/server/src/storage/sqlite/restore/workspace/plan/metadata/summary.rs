/**
@module PROJECTOR.SERVER.SQLITE_WORKSPACE_RESTORE_SUMMARY
Owns summary rendering for classified SQLite workspace-restore changes.
*/
// @fileimplements PROJECTOR.SERVER.SQLITE_WORKSPACE_RESTORE_SUMMARY
use projector_domain::ManifestEntry;

use super::classify::RestoreChangeClass;

pub(super) fn restore_change_summary(
    change: &RestoreChangeClass,
    restored_entry: &ManifestEntry,
    target_cursor: u64,
) -> String {
    let path_display = format!(
        "{}/{}",
        restored_entry.mount_relative_path.display(),
        restored_entry.relative_path.display()
    );

    match change {
        RestoreChangeClass::Deleted => format!(
            "workspace restore to cursor {target_cursor} removed text document from live workspace at {path_display}"
        ),
        RestoreChangeClass::RestoredLive => format!(
            "workspace restore to cursor {target_cursor} restored text document at {path_display}"
        ),
        RestoreChangeClass::Moved => format!(
            "workspace restore to cursor {target_cursor} moved text document to {path_display}"
        ),
        RestoreChangeClass::BodyRestored => format!(
            "workspace restore to cursor {target_cursor} restored text document body at {path_display}"
        ),
    }
}
