/**
@module PROJECTOR.EDGE.DOCTOR_SYNC_ENTRY_CHECKS
Owns doctor-time validation and rendering for repo-local sync entries, including gitignore, tracked-file, and profile-reference checks.
*/
// @fileimplements PROJECTOR.EDGE.DOCTOR_SYNC_ENTRY_CHECKS
use std::error::Error;

use crate::cli_support::{format_sync_entry_kind, is_path_gitignored, is_path_tracked_by_git};

use super::DoctorContext;
use super::report::DoctorFinding;

pub(super) fn collect_sync_entry_findings(
    context: &DoctorContext,
    findings: &mut Vec<DoctorFinding>,
) -> Result<(), Box<dyn Error>> {
    if !context.sync_config.entries.is_empty() && !context.repo_registered() {
        findings.push(DoctorFinding::warning(
            "repo has sync entries but is not registered in the machine sync registry".to_owned(),
        ));
    }
    if context.runtime_status.sync_issue_count > 0 {
        findings.push(DoctorFinding::warning(format!(
            "repo has {} recent sync issue(s)",
            context.runtime_status.sync_issue_count
        )));
    }

    for entry in &context.sync_config.entries {
        let gitignored = is_path_gitignored(&context.repo_root, &entry.local_relative_path)?;
        let tracked = is_path_tracked_by_git(&context.repo_root, &entry.local_relative_path)?;
        let local_exists = context.repo_root.join(&entry.local_relative_path).exists();
        let profile_registered = context
            .profile_registry
            .profiles
            .iter()
            .any(|profile| profile.profile_id == entry.server_profile_id);

        if !gitignored {
            findings.push(DoctorFinding::error(format!(
                "sync entry {} is not gitignored",
                entry.local_relative_path.display()
            )));
        }
        if tracked {
            findings.push(DoctorFinding::error(format!(
                "sync entry {} is already tracked by git",
                entry.local_relative_path.display()
            )));
        }
        if !profile_registered {
            findings.push(DoctorFinding::error(format!(
                "sync entry {} refers to unknown server profile {}",
                entry.local_relative_path.display(),
                entry.server_profile_id
            )));
        }

        println!(
            "sync_entry_check: path={} kind={} profile={} profile_registered={} gitignored={} tracked={} local_exists={}",
            entry.local_relative_path.display(),
            format_sync_entry_kind(&entry.kind),
            entry.server_profile_id,
            profile_registered,
            gitignored,
            tracked,
            local_exists
        );
    }

    Ok(())
}
