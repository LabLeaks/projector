/**
@module PROJECTOR.RUNTIME.WATCH_SYNC
Coordinates watcher-driven runtime reconciliation by delegating path move detection and watcher mutation application to narrower runtime modules.
*/
// @fileimplements PROJECTOR.RUNTIME.WATCH_SYNC
use std::error::Error;
use std::io;

use projector_domain::{BootstrapSnapshot, SyncContext};

use super::super::SyncRunner;
use crate::{Transport, WatcherEvent};

mod moves;
mod mutations;

pub(super) fn push_watcher_events<C, T>(
    runner: &mut SyncRunner<'_, C, T>,
    transport: &mut T,
    current_snapshot: &BootstrapSnapshot,
    current_cursor: u64,
    events: &[WatcherEvent],
) -> Result<Option<(BootstrapSnapshot, u64)>, Box<dyn Error>>
where
    C: SyncContext,
    T: Transport<Error = io::Error>,
{
    let touched_paths = moves::touched_projection_paths(runner.binding, events);
    if touched_paths.is_empty() {
        return Ok(None);
    }

    let move_operations = moves::detect_touched_path_moves(current_snapshot, &touched_paths)?;
    mutations::apply_watcher_mutations(
        runner,
        transport,
        current_snapshot,
        current_cursor,
        touched_paths,
        move_operations,
    )
}
