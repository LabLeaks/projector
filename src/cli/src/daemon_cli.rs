/**
@module PROJECTOR.EDGE.DAEMON_CLI
Owns machine-daemon lifecycle commands and the internal daemon process entrypoint.
*/
// @fileimplements PROJECTOR.EDGE.DAEMON_CLI
use std::env;
use std::error::Error;
use std::fs;
use std::path::Path;
use std::process::{self, Command};
use std::thread;
use std::time::Duration;

use projector_runtime::{
    FileMachineDaemonStateStore, FileMachineSyncRegistryStore, FileRepoSyncConfigStore,
    MachineDaemonOptions, ProjectorHome, discover_repo_root, run_machine_daemon, terminate_process,
};

pub(crate) fn run_start(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    if !args.is_empty() {
        return Err("start does not accept arguments".into());
    }
    run_start_current_repo()
}

pub(crate) fn run_stop(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        None => run_stop_current_repo(),
        Some("--all") if args.len() == 1 => run_stop_all(),
        _ => Err("usage: projector stop [--all]".into()),
    }
}

pub(crate) fn run_daemon(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("run") => {
            let poll_ms = env::var("PROJECTOR_DAEMON_POLL_MS")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(1_000);
            let cycles = env::var("PROJECTOR_DAEMON_CYCLES")
                .ok()
                .and_then(|value| value.parse::<usize>().ok());
            let home = ProjectorHome::discover()?;
            run_machine_daemon(home, &MachineDaemonOptions { poll_ms, cycles })
        }
        _ => Err("usage: projector daemon run".into()),
    }
}

fn run_start_current_repo() -> Result<(), Box<dyn Error>> {
    let home = ProjectorHome::discover()?;
    register_current_repo_if_configured()?;
    let daemon_state_store = FileMachineDaemonStateStore::new(home.clone());
    if let Some(active) = daemon_state_store.load_active()? {
        println!("daemon_running: true");
        println!("projector_home: {}", home.root().display());
        println!("daemon_pid: {}", active.pid);
        println!("daemon_started_at_ms: {}", active.started_at_ms);
        print_current_repo_sync_state(&home)?;
        return Ok(());
    }

    let exe = env::current_exe()?;
    let mut child = Command::new(exe);
    child
        .arg("daemon")
        .arg("run")
        .stdin(process::Stdio::null())
        .stdout(process::Stdio::null())
        .stderr(process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        unsafe {
            child.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    let _child = child.spawn()?;

    for _ in 0..20 {
        thread::sleep(Duration::from_millis(50));
        if let Some(active) = daemon_state_store.load_active()? {
            println!("daemon_running: true");
            println!("projector_home: {}", home.root().display());
            println!("daemon_pid: {}", active.pid);
            println!("daemon_started_at_ms: {}", active.started_at_ms);
            print_current_repo_sync_state(&home)?;
            return Ok(());
        }
    }

    Err("machine daemon did not report healthy startup".into())
}

fn register_current_repo_if_configured() -> Result<(), Box<dyn Error>> {
    let cwd = env::current_dir()?;
    let repo_root = discover_repo_root(&cwd);
    let config = FileRepoSyncConfigStore::new(&repo_root).load()?;
    if config.entries.is_empty() {
        return Ok(());
    }

    let home = ProjectorHome::discover()?;
    let registry_store = FileMachineSyncRegistryStore::new(home);
    let _ = registry_store.sync_repo(&repo_root, &config)?;
    Ok(())
}

fn run_stop_current_repo() -> Result<(), Box<dyn Error>> {
    let home = ProjectorHome::discover()?;
    let cwd = env::current_dir()?;
    let repo_root = discover_repo_root(&cwd);
    let entry_count = FileRepoSyncConfigStore::new(&repo_root)
        .load()
        .map(|config| config.entries.len())
        .ok();
    let registry_store = FileMachineSyncRegistryStore::new(home.clone());
    let _ = registry_store.unregister_repo(&repo_root)?;
    let active = FileMachineDaemonStateStore::new(home.clone()).load_active()?;

    println!("repo_syncing: false");
    println!("repo_root: {}", repo_root.display());
    match entry_count {
        Some(entry_count) => println!("repo_sync_entry_count: {entry_count}"),
        None => println!("repo_sync_entry_count: unknown"),
    }
    println!("daemon_running: {}", active.is_some());
    println!("projector_home: {}", home.root().display());
    if let Some(active) = active {
        println!("daemon_pid: {}", active.pid);
        println!("daemon_started_at_ms: {}", active.started_at_ms);
    }
    Ok(())
}

fn run_stop_all() -> Result<(), Box<dyn Error>> {
    let home = ProjectorHome::discover()?;
    let daemon_state_store = FileMachineDaemonStateStore::new(home);
    let Some(active) = daemon_state_store.load_active()? else {
        println!("daemon_running: false");
        return Ok(());
    };

    terminate_process(active.pid)?;
    for _ in 0..20 {
        thread::sleep(Duration::from_millis(50));
        if daemon_state_store.load_active()?.is_none() {
            println!("daemon_running: false");
            return Ok(());
        }
    }

    Err("machine daemon did not stop cleanly".into())
}

pub(crate) fn current_repo_syncing(home: &ProjectorHome) -> Result<bool, Box<dyn Error>> {
    let cwd = env::current_dir()?;
    let repo_root = discover_repo_root(&cwd);
    let registry = FileMachineSyncRegistryStore::new(home.clone()).load()?;
    Ok(registry
        .repos
        .iter()
        .any(|repo| same_repo_root(&repo.repo_root, &repo_root)))
}

fn print_current_repo_sync_state(home: &ProjectorHome) -> Result<(), Box<dyn Error>> {
    let cwd = env::current_dir()?;
    let repo_root = discover_repo_root(&cwd);
    let repo_syncing = current_repo_syncing(home)?;
    println!("repo_syncing: {repo_syncing}");
    println!("repo_root: {}", repo_root.display());
    match FileRepoSyncConfigStore::new(&repo_root).load() {
        Ok(config) => println!("repo_sync_entry_count: {}", config.entries.len()),
        Err(_) => println!("repo_sync_entry_count: unknown"),
    }
    Ok(())
}

fn same_repo_root(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    match (fs::canonicalize(left), fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}
