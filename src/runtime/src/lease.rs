/**
@module PROJECTOR.RUNTIME.LEASE
Owns repo-local checkout runtime lease acquisition, stale-lock detection, and active-watch inspection for foreground sync loops.
*/
// @fileimplements PROJECTOR.RUNTIME.LEASE
use std::fs::{self, OpenOptions};
use std::io::{self, ErrorKind, Write};
use std::path::PathBuf;
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActiveRuntimeLease {
    pub pid: u32,
    pub started_at_ms: u128,
}

#[derive(Clone, Debug)]
pub struct FileRuntimeLeaseStore {
    path: PathBuf,
}

impl FileRuntimeLeaseStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn load_active(&self) -> Result<Option<ActiveRuntimeLease>, io::Error> {
        let Some(lease) = self.load_from_disk()? else {
            return Ok(None);
        };

        if process_is_running(lease.pid) {
            return Ok(Some(lease));
        }

        match fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(err) if err.kind() == ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }

        Ok(None)
    }

    pub fn acquire(&self) -> Result<RuntimeLeaseGuard, io::Error> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }

        let desired = ActiveRuntimeLease {
            pid: process::id(),
            started_at_ms: now_ms(),
        };

        loop {
            match self.try_create(&desired) {
                Ok(guard) => return Ok(guard),
                Err(err) if err.kind() == ErrorKind::AlreadyExists => {
                    if let Some(existing) = self.load_active()? {
                        return Err(io::Error::new(
                            ErrorKind::AlreadyExists,
                            format!(
                                "checkout runtime already active: pid={} started_at_ms={}",
                                existing.pid, existing.started_at_ms
                            ),
                        ));
                    }
                }
                Err(err) => return Err(err),
            }
        }
    }

    fn try_create(&self, lease: &ActiveRuntimeLease) -> Result<RuntimeLeaseGuard, io::Error> {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&self.path)?;
        file.write_all(serialize_lease(lease).as_bytes())?;
        Ok(RuntimeLeaseGuard {
            path: self.path.clone(),
            lease: lease.clone(),
        })
    }

    fn load_from_disk(&self) -> Result<Option<ActiveRuntimeLease>, io::Error> {
        if !self.path.exists() {
            return Ok(None);
        }

        let content = fs::read_to_string(&self.path)?;
        let mut pid = None;
        let mut started_at_ms = None;

        for line in content.lines() {
            if let Some(value) = line.strip_prefix("pid=") {
                pid = value.parse::<u32>().ok();
            } else if let Some(value) = line.strip_prefix("started_at_ms=") {
                started_at_ms = value.parse::<u128>().ok();
            }
        }

        match (pid, started_at_ms) {
            (Some(pid), Some(started_at_ms)) => Ok(Some(ActiveRuntimeLease { pid, started_at_ms })),
            _ => Ok(None),
        }
    }
}

#[derive(Debug)]
pub struct RuntimeLeaseGuard {
    path: PathBuf,
    lease: ActiveRuntimeLease,
}

impl RuntimeLeaseGuard {
    pub fn lease(&self) -> &ActiveRuntimeLease {
        &self.lease
    }
}

impl Drop for RuntimeLeaseGuard {
    fn drop(&mut self) {
        let current = fs::read_to_string(&self.path).ok();
        let still_owned = current
            .as_deref()
            .map(|content| content == serialize_lease(&self.lease))
            .unwrap_or(false);

        if still_owned {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn serialize_lease(lease: &ActiveRuntimeLease) -> String {
    format!("pid={}\nstarted_at_ms={}\n", lease.pid, lease.started_at_ms)
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_millis()
}

#[cfg(unix)]
fn process_is_running(pid: u32) -> bool {
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if result == 0 {
        return true;
    }

    matches!(io::Error::last_os_error().raw_os_error(), Some(libc::EPERM))
}

#[cfg(not(unix))]
fn process_is_running(_pid: u32) -> bool {
    true
}
