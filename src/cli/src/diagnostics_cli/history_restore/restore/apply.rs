/**
@module PROJECTOR.EDGE.RESTORE_APPLY
Owns restore preview rendering, revision existence validation, remote restore writes, and local rematerialization.
*/
// @fileimplements PROJECTOR.EDGE.RESTORE_APPLY
use std::error::Error;
use std::path::Path;

use projector_runtime::Transport;

use crate::sync_entry_cli::materialize_sync_config_entries;

use super::super::render::print_restore_preview;
use super::prepare::PreparedRestore;

pub(super) fn apply_or_preview_restore(
    mut prepared: PreparedRestore,
    restore_seq: u64,
) -> Result<(), Box<dyn Error>> {
    let target_revision = prepared
        .all_revisions
        .iter()
        .find(|revision| revision.seq == restore_seq)
        .ok_or_else(|| {
            format!(
                "document at {} does not have body revision {}",
                prepared.requested_path.display(),
                restore_seq
            )
        })?;

    print_restore_preview(
        &prepared.requested_path,
        &prepared.document_id,
        restore_seq,
        &prepared.current_text,
        &target_revision.body_text,
    );

    if should_apply(&prepared) {
        let (target_mount_relative_path, target_relative_path) =
            target_override_refs(prepared.apply_target_override.as_ref());
        prepared.transport.restore_document_body_revision(
            &prepared.binding,
            &prepared.document_id,
            restore_seq,
            target_mount_relative_path,
            target_relative_path,
        )?;

        materialize_sync_config_entries(
            &prepared.repo_root,
            &prepared.sync_config,
            &prepared.profiles,
        )?;
        println!("applied: true");
    } else {
        println!("applied: false");
        println!("next: rerun with --confirm to apply this restore");
    }
    Ok(())
}

fn should_apply(prepared: &PreparedRestore) -> bool {
    super::super::args::should_use_restore_browser(&prepared.restore_args)
        || prepared.restore_args.confirm
}

fn target_override_refs(
    override_paths: Option<&(std::path::PathBuf, std::path::PathBuf)>,
) -> (Option<&Path>, Option<&Path>) {
    match override_paths {
        Some((mount_relative_path, relative_path)) => (
            Some(mount_relative_path.as_path()),
            Some(relative_path.as_path()),
        ),
        None => (None, None),
    }
}
