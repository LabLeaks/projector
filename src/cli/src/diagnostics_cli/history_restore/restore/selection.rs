/**
@module PROJECTOR.EDGE.RESTORE_SELECTION
Owns interactive and scripted restore revision selection, including terminal-browser cancellation and default-previous behavior.
*/
// @fileimplements PROJECTOR.EDGE.RESTORE_SELECTION
use std::error::Error;

use crate::restore_browser::{RestoreBrowserExit, browse_restore_revisions};

use super::super::args::{default_restore_seq, resolve_restore_seq, should_use_restore_browser};
use super::prepare::PreparedRestore;

pub(super) fn select_restore_revision(
    prepared: PreparedRestore,
) -> Result<Option<(PreparedRestore, u64)>, Box<dyn Error>> {
    let interactive_restore = should_use_restore_browser(&prepared.restore_args);
    if !interactive_restore
        && prepared.restore_args.confirm
        && prepared.restore_args.selector.is_none()
    {
        return Err("--confirm outside interactive restore requires --seq".into());
    }

    let restore_seq = if interactive_restore {
        match browse_restore_revisions(
            &prepared.requested_path,
            &prepared.current_text,
            &prepared.all_revisions,
            default_restore_seq(&prepared.all_revisions, &prepared.requested_path)?,
            false,
        )? {
            RestoreBrowserExit::Selected(selection) => Some(selection.seq),
            RestoreBrowserExit::Cancelled { selected_seq } => {
                println!("restore: cancelled");
                println!("selected_seq: {}", selected_seq);
                None
            }
        }
    } else {
        Some(resolve_restore_seq(
            &prepared.restore_args,
            &prepared.all_revisions,
            &prepared.requested_path,
        )?)
    };

    Ok(restore_seq.map(|seq| (prepared, seq)))
}
