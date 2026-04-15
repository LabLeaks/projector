/**
@module PROJECTOR.EDGE.DOCTOR_PROFILE_CHECKS
Owns doctor-time validation and rendering for referenced server profiles, including registration and reachability checks.
*/
// @fileimplements PROJECTOR.EDGE.DOCTOR_PROFILE_CHECKS
use crate::connection_cli::server_addr_reachable;

use super::DoctorContext;
use super::report::DoctorFinding;

pub(super) fn collect_profile_findings(context: &DoctorContext, findings: &mut Vec<DoctorFinding>) {
    for profile_id in context.referenced_profiles() {
        let registered = context
            .profile_registry
            .profiles
            .iter()
            .find(|profile| profile.profile_id == profile_id);
        let reachable = registered
            .map(|profile| server_addr_reachable(&profile.server_addr))
            .map(|reachable| reachable.to_string())
            .unwrap_or_else(|| "unknown".to_owned());
        let server_addr = registered
            .map(|profile| profile.server_addr.as_str())
            .unwrap_or("unknown");
        let ssh_target = registered
            .and_then(|profile| profile.ssh_target.as_deref())
            .unwrap_or("unknown");

        match (registered.is_some(), reachable.as_str()) {
            (false, _) => findings.push(DoctorFinding::error(format!(
                "server profile {profile_id} is not registered"
            ))),
            (true, "false") => findings.push(DoctorFinding::warning(format!(
                "server profile {profile_id} is not reachable"
            ))),
            _ => {}
        }

        println!(
            "profile_check: profile={} registered={} reachable={} server_addr={} ssh_target={}",
            profile_id,
            registered.is_some(),
            reachable,
            server_addr,
            ssh_target
        );
    }
}
