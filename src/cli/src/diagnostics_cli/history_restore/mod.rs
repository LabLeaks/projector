/**
@module PROJECTOR.EDGE.HISTORY_RESTORE_CLI
Coordinates history inspection and restore flows by delegating argument parsing, history rendering, restore execution, and shared formatting to narrower edge modules.
*/
// @fileimplements PROJECTOR.EDGE.HISTORY_RESTORE_CLI
mod args;
mod history;
mod render;
mod restore;

pub(crate) use history::{resolve_live_entry_for_repo_relative_path, run_history};
pub(crate) use render::format_event_path;
pub(crate) use restore::run_restore;
