/**
@module PROJECTOR.EDGE.RESTORE_CLI
Coordinates interactive and scripted document restore flows by delegating restore preparation, revision selection, and apply/rematerialize work to narrower restore helpers.
*/
// @fileimplements PROJECTOR.EDGE.RESTORE_CLI
use std::error::Error;

mod apply;
mod prepare;
mod selection;

use super::args::parse_restore_args;
use apply::apply_or_preview_restore;
use prepare::prepare_restore;
use selection::select_restore_revision;

pub(crate) fn run_restore(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    let restore_args = parse_restore_args(&args)?;
    let Some((prepared, restore_seq)) = select_restore_revision(prepare_restore(restore_args)?)?
    else {
        return Ok(());
    };

    apply_or_preview_restore(prepared, restore_seq)
}
