/**
@module PROJECTOR.EDGE.HISTORY_SURGERY_CLI
Owns scripted retained-history redaction and purge flows by resolving a repo-relative path to one live bound document, previewing the retained impact, and requiring `--confirm` to apply surgery.
*/
// @fileimplements PROJECTOR.EDGE.HISTORY_SURGERY_CLI
use std::error::Error;

use projector_runtime::{HttpTransport, Transport};

use crate::cli_support::{normalize_projection_relative_path, repo_root};
use crate::sync_entry_cli::{
    load_sync_targets_with_profiles, resolve_document_id_for_requested_path,
    resolve_sync_target_for_requested_path, workspace_binding_for_target,
};

use super::args::{parse_purge_args, parse_redact_args};

pub(crate) fn run_redact(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    let redact_args = parse_redact_args(&args)?;
    let mut prepared = prepare_document_history_surgery(&redact_args.repo_relative_path)?;
    let revisions = prepared
        .transport
        .list_body_revisions(&prepared.binding, &prepared.document_id, 20)?;
    let matching_revisions = revisions
        .iter()
        .filter(|revision| {
            revision.base_text.contains(&redact_args.exact_text)
                || revision.body_text.contains(&redact_args.exact_text)
        })
        .count();
    if matching_revisions == 0 {
        return Err(format!(
            "no retained revisions for {} contain the exact text {:?}",
            prepared.requested_path.display(),
            redact_args.exact_text
        )
        .into());
    }

    println!("path: {}", prepared.requested_path.display());
    println!("document_id: {}", prepared.document_id.as_str());
    println!("matching_revisions: {matching_revisions}");
    println!("replacement: [REDACTED]");

    if !redact_args.confirm {
        println!("next: rerun with --confirm to apply this redaction");
        return Ok(());
    }

    prepared.transport.redact_document_body_history(
        &prepared.binding,
        &prepared.document_id,
        &redact_args.exact_text,
    )?;
    println!("redaction: applied");
    Ok(())
}

pub(crate) fn run_purge(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    let purge_args = parse_purge_args(&args)?;
    let mut prepared = prepare_document_history_surgery(&purge_args.repo_relative_path)?;
    let revisions = prepared
        .transport
        .list_body_revisions(&prepared.binding, &prepared.document_id, 20)?;
    if revisions.is_empty() {
        return Err(format!(
            "document at {} does not have any retained body revisions",
            prepared.requested_path.display()
        )
        .into());
    }
    let clearable_revisions = revisions
        .iter()
        .filter(|revision| !revision.base_text.is_empty() || !revision.body_text.is_empty())
        .count();

    println!("path: {}", prepared.requested_path.display());
    println!("document_id: {}", prepared.document_id.as_str());
    println!("retained_revisions: {}", revisions.len());
    println!("clearable_revisions: {clearable_revisions}");

    if !purge_args.confirm {
        println!("next: rerun with --confirm to purge retained history");
        return Ok(());
    }

    prepared
        .transport
        .purge_document_body_history(&prepared.binding, &prepared.document_id)?;
    println!("purge: applied");
    Ok(())
}

struct PreparedHistorySurgery {
    requested_path: std::path::PathBuf,
    binding: projector_domain::CheckoutBinding,
    document_id: projector_domain::DocumentId,
    transport: HttpTransport,
}

fn prepare_document_history_surgery(
    repo_relative_path: &str,
) -> Result<PreparedHistorySurgery, Box<dyn Error>> {
    let repo_root = repo_root()?;
    let requested_path = normalize_projection_relative_path(repo_relative_path)?;
    let sync_targets = load_sync_targets_with_profiles(&repo_root)?;
    let (target, mount_relative_path, relative_path) =
        resolve_sync_target_for_requested_path(&requested_path, &sync_targets)?;
    let binding = workspace_binding_for_target(target, &sync_targets)?;
    let server_addr = binding
        .server_addr
        .as_deref()
        .ok_or("history surgery requires a server-bound sync entry")?;
    let mut transport = HttpTransport::new(format!("http://{server_addr}"));
    let (snapshot, _) = transport.bootstrap(&binding)?;
    let document_id = resolve_document_id_for_requested_path(
        &mut transport,
        &binding,
        &snapshot,
        &requested_path,
        &mount_relative_path,
        &relative_path,
    )?;
    Ok(PreparedHistorySurgery {
        requested_path,
        binding,
        document_id,
        transport,
    })
}
