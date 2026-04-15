/**
@module PROJECTOR.EDGE.CONNECTION_PROFILES_CLI
Owns human and script-facing connection status, add/update, disconnect warnings, and profile selection for dependent commands.
*/
// @fileimplements PROJECTOR.EDGE.CONNECTION_PROFILES_CLI
use std::error::Error;
use std::io::IsTerminal;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use projector_domain::SyncEntryKind;
use projector_runtime::{
    FileMachineSyncRegistryStore, FileRepoSyncConfigStore, FileServerProfileStore, ProjectorHome,
    ServerProfile,
};

use crate::cli_support::format_sync_entry_kind;

use super::args::{parse_connect_args, parse_disconnect_args};
use super::prompts::{fill_connect_defaults, prompt_confirm, prompt_required};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProfileDependentEntry {
    pub(crate) repo_root: PathBuf,
    pub(crate) local_relative_path: PathBuf,
    pub(crate) kind: SyncEntryKind,
}

pub(crate) fn run_connect(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    let home = ProjectorHome::discover()?;
    let profiles = FileServerProfileStore::new(home.clone());
    if matches!(args.first().map(String::as_str), Some("status")) {
        if args.len() != 1 {
            return Err("usage: projector connect status".into());
        }
        let registry = profiles.load()?;
        println!("connection_count: {}", registry.profiles.len());
        for profile in registry.profiles {
            let dependents = collect_profile_dependents(&home, &profile.profile_id)?;
            let mut repo_roots = dependents
                .iter()
                .map(|dependent| dependent.repo_root.clone())
                .collect::<Vec<_>>();
            repo_roots.sort();
            repo_roots.dedup();
            println!(
                "connection: id={} server_addr={} ssh_target={} reachable={} repo_count={} sync_entry_count={}",
                profile.profile_id,
                profile.server_addr,
                profile.ssh_target.as_deref().unwrap_or("none"),
                server_addr_reachable(&profile.server_addr),
                repo_roots.len(),
                dependents.len()
            );
            for dependent in dependents {
                println!(
                    "connection_sync_entry: id={} repo={} path={} kind={}",
                    profile.profile_id,
                    dependent.repo_root.display(),
                    dependent.local_relative_path.display(),
                    format_sync_entry_kind(&dependent.kind)
                );
            }
        }
        return Ok(());
    }

    let interactive = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let connect_args = fill_connect_defaults(parse_connect_args(&args)?, interactive)?;
    let profile_id = connect_args
        .profile_id
        .as_deref()
        .expect("profile id present");
    let server_addr = connect_args
        .server_addr
        .as_deref()
        .expect("server addr present");
    let existed = profiles.resolve_profile(profile_id)?.is_some();
    let registry =
        profiles.upsert_profile(profile_id, server_addr, connect_args.ssh_target.as_deref())?;
    println!("connection: {}", if existed { "updated" } else { "added" });
    println!("profile: {profile_id}");
    println!("server_addr: {server_addr}");
    println!(
        "ssh_target: {}",
        connect_args.ssh_target.as_deref().unwrap_or("none")
    );
    println!("connection_count: {}", registry.profiles.len());
    Ok(())
}

pub(crate) fn run_disconnect(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    let disconnect_args = parse_disconnect_args(&args)?;
    let home = ProjectorHome::discover()?;
    let profiles = FileServerProfileStore::new(home.clone());
    let profile = profiles
        .resolve_profile(&disconnect_args.profile_id)?
        .ok_or_else(|| {
            format!(
                "server profile {} is not registered",
                disconnect_args.profile_id
            )
        })?;
    let affected = collect_profile_dependents(&home, &disconnect_args.profile_id)?;

    println!("disconnect_profile: {}", profile.profile_id);
    println!("server_addr: {}", profile.server_addr);
    println!(
        "ssh_target: {}",
        profile.ssh_target.as_deref().unwrap_or("none")
    );
    println!("affected_sync_entry_count: {}", affected.len());
    for dependent in &affected {
        println!(
            "affected_sync_entry: repo={} path={} kind={}",
            dependent.repo_root.display(),
            dependent.local_relative_path.display(),
            format_sync_entry_kind(&dependent.kind)
        );
    }

    if !disconnect_args.yes {
        if !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) {
            return Err("disconnect requires --yes outside an interactive terminal".into());
        }
        if !prompt_confirm(
            &format!(
                "Disconnect profile {} and leave these paths desynced? [y/N]: ",
                profile.profile_id
            ),
            false,
        )? {
            println!("disconnect: cancelled");
            return Ok(());
        }
    }

    profiles.remove_profile(&disconnect_args.profile_id)?;
    println!("disconnect: complete");
    println!("profile: {}", disconnect_args.profile_id);
    Ok(())
}

pub(crate) fn collect_profile_dependents(
    home: &ProjectorHome,
    profile_id: &str,
) -> Result<Vec<ProfileDependentEntry>, Box<dyn Error>> {
    let registry = FileMachineSyncRegistryStore::new(home.clone()).load()?;
    let mut dependents = Vec::new();
    for repo in registry.repos {
        let config = FileRepoSyncConfigStore::new(&repo.repo_root).load()?;
        for entry in config
            .entries
            .into_iter()
            .filter(|entry| entry.server_profile_id == profile_id)
        {
            dependents.push(ProfileDependentEntry {
                repo_root: repo.repo_root.clone(),
                local_relative_path: entry.local_relative_path,
                kind: entry.kind,
            });
        }
    }
    dependents.sort_by(|left, right| {
        left.repo_root
            .cmp(&right.repo_root)
            .then_with(|| left.local_relative_path.cmp(&right.local_relative_path))
    });
    Ok(dependents)
}

pub(crate) fn resolve_profile_for_action(
    profiles: &FileServerProfileStore,
    explicit_profile_id: Option<&str>,
    command_name: &str,
) -> Result<ServerProfile, Box<dyn Error>> {
    if let Some(profile_id) = explicit_profile_id {
        return profiles
            .resolve_profile(profile_id)?
            .ok_or_else(|| format!("server profile {profile_id} is not registered").into());
    }

    let registry = profiles.load()?;
    match registry.profiles.as_slice() {
        [] => Err("no server profiles are connected; run `projector connect` first".into()),
        [profile] => Ok(profile.clone()),
        profiles_list => {
            if !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) {
                return Err(format!(
                    "multiple server profiles are connected; rerun `{command_name} --profile <id> ...`"
                )
                .into());
            }
            println!("connected_profiles:");
            for profile in profiles_list {
                println!(
                    "profile: id={} server_addr={} ssh_target={}",
                    profile.profile_id,
                    profile.server_addr,
                    profile.ssh_target.as_deref().unwrap_or("none")
                );
            }
            let profile_id = prompt_required("Profile id")?;
            profiles
                .resolve_profile(&profile_id)?
                .ok_or_else(|| format!("server profile {profile_id} is not registered").into())
        }
    }
}

pub(crate) fn server_addr_reachable(server_addr: &str) -> bool {
    for _ in 0..2 {
        let Ok(addrs) = server_addr.to_socket_addrs() else {
            continue;
        };
        for addr in addrs {
            if TcpStream::connect_timeout(&addr, Duration::from_millis(750)).is_ok() {
                return true;
            }
        }
        thread::sleep(Duration::from_millis(100));
    }
    false
}

pub(super) fn wait_for_server_reachability(
    server_addr: &str,
    attempts: usize,
    delay: Duration,
) -> bool {
    for _ in 0..attempts {
        if server_addr_reachable(server_addr) {
            return true;
        }
        thread::sleep(delay);
    }
    false
}
