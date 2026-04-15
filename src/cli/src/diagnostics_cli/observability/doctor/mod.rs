/**
@module PROJECTOR.EDGE.DOCTOR_CLI
Coordinates the explicit setup and sanity audit surface by delegating profile checks, sync-entry checks, and final reporting to narrower doctor helpers.
*/
// @fileimplements PROJECTOR.EDGE.DOCTOR_CLI
use std::collections::BTreeSet;
use std::error::Error;
use std::path::PathBuf;

use projector_runtime::{
    FileMachineDaemonStateStore, FileMachineSyncRegistryStore, FileRuntimeLeaseStore,
    FileRuntimeStatusStore, FileServerProfileStore, MachineDaemonState, MachineSyncRegistry,
    ProjectorHome, RuntimeStatus, ServerProfileRegistry,
};

use crate::cli_support::repo_root;
use crate::sync_entry_cli::load_sync_config;

mod entries;
mod profiles;
mod report;

use entries::collect_sync_entry_findings;
use profiles::collect_profile_findings;
use report::{DoctorFinding, print_summary};

pub(crate) fn run_doctor() -> Result<(), Box<dyn Error>> {
    let context = DoctorContext::load()?;
    let mut findings = Vec::<DoctorFinding>::new();

    print_header(&context);
    collect_profile_findings(&context, &mut findings);
    collect_sync_entry_findings(&context, &mut findings)?;
    print_summary(&context, findings);
    Ok(())
}

pub(super) struct DoctorContext {
    pub(super) repo_root: PathBuf,
    pub(super) profile_registry: ServerProfileRegistry,
    pub(super) sync_config: projector_domain::RepoSyncConfig,
    pub(super) machine_registry: MachineSyncRegistry,
    pub(super) machine_daemon: Option<MachineDaemonState>,
    pub(super) runtime_lease_active: bool,
    pub(super) runtime_status: RuntimeStatus,
}

impl DoctorContext {
    fn load() -> Result<Self, Box<dyn Error>> {
        let repo_root = repo_root()?;
        let home = ProjectorHome::discover()?;
        let profile_registry = FileServerProfileStore::new(home.clone()).load()?;
        let sync_config = load_sync_config(&repo_root)?;
        let machine_registry = FileMachineSyncRegistryStore::new(home.clone()).load()?;
        let machine_daemon = FileMachineDaemonStateStore::new(home.clone()).load_active()?;
        let runtime_lease_active =
            FileRuntimeLeaseStore::new(repo_root.join(".projector/runtime.lock"))
                .load_active()?
                .is_some();
        let runtime_status =
            FileRuntimeStatusStore::new(repo_root.join(".projector/status.txt")).load()?;

        Ok(Self {
            repo_root,
            profile_registry,
            sync_config,
            machine_registry,
            machine_daemon,
            runtime_lease_active,
            runtime_status,
        })
    }

    pub(super) fn repo_registered(&self) -> bool {
        self.machine_registry
            .repos
            .iter()
            .any(|registered| registered.repo_root == self.repo_root)
    }

    pub(super) fn referenced_profiles(&self) -> BTreeSet<String> {
        self.sync_config
            .entries
            .iter()
            .map(|entry| entry.server_profile_id.clone())
            .collect()
    }
}

fn print_header(context: &DoctorContext) {
    println!(
        "connected_profile_count: {}",
        context.profile_registry.profiles.len()
    );
    println!(
        "machine_daemon_running: {}",
        context.machine_daemon.is_some()
    );
    println!("repo_registered: {}", context.repo_registered());
    println!("runtime_lease_active: {}", context.runtime_lease_active);
    println!(
        "recent_sync_issue_count: {}",
        context.runtime_status.sync_issue_count
    );
    println!("sync_entry_count: {}", context.sync_config.entries.len());
}
