/**
@module PROJECTOR.EDGE.DAEMON_CLI
Owns machine-daemon lifecycle commands and the internal daemon process entrypoint.
*/
// @fileimplements PROJECTOR.EDGE.DAEMON_CLI
use std::env;
use std::error::Error;
use std::process::{self, Command};
use std::thread;
use std::time::Duration;

use projector_runtime::{
    FileMachineDaemonStateStore, MachineDaemonOptions, ProjectorHome, run_machine_daemon,
    terminate_process,
};

pub(crate) fn run_sync_command(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("start") if args.len() == 1 => run_sync_start(),
        Some("stop") if args.len() == 1 => run_sync_stop(),
        Some("status") if args.len() == 1 => run_sync_status(),
        Some("start" | "stop" | "status") => {
            Err("sync start|stop|status do not accept additional arguments".into())
        }
        _ => Err("usage: projector sync <start|stop|status>".into()),
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

fn run_sync_start() -> Result<(), Box<dyn Error>> {
    let home = ProjectorHome::discover()?;
    let daemon_state_store = FileMachineDaemonStateStore::new(home.clone());
    if let Some(active) = daemon_state_store.load_active()? {
        println!("daemon_running: true");
        println!("projector_home: {}", home.root().display());
        println!("daemon_pid: {}", active.pid);
        println!("daemon_started_at_ms: {}", active.started_at_ms);
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
            return Ok(());
        }
    }

    Err("machine daemon did not report healthy startup".into())
}

fn run_sync_stop() -> Result<(), Box<dyn Error>> {
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

fn run_sync_status() -> Result<(), Box<dyn Error>> {
    let home = ProjectorHome::discover()?;
    let daemon_state_store = FileMachineDaemonStateStore::new(home.clone());
    if let Some(active) = daemon_state_store.load_active()? {
        println!("daemon_running: true");
        println!("projector_home: {}", home.root().display());
        println!("daemon_pid: {}", active.pid);
        println!("daemon_started_at_ms: {}", active.started_at_ms);
    } else {
        println!("daemon_running: false");
        println!("projector_home: {}", home.root().display());
    }
    Ok(())
}
