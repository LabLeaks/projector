/**
@module PROJECTOR.RUNTIME.MACHINE_DAEMON_STATE
Owns persistence and liveness checks for the machine-global daemon process so CLI control commands can observe and stop the current coordinator safely.
*/
// @fileimplements PROJECTOR.RUNTIME.MACHINE_DAEMON_STATE
use std::fs;
use std::io;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::ProjectorHome;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MachineDaemonState {
    pub pid: u32,
    pub started_at_ms: u128,
    pub heartbeat_at_ms: u128,
}

#[derive(Clone, Debug)]
pub struct FileMachineDaemonStateStore {
    home: ProjectorHome,
}

impl FileMachineDaemonStateStore {
    pub fn new(home: ProjectorHome) -> Self {
        Self { home }
    }

    pub fn load(&self) -> Result<Option<MachineDaemonState>, io::Error> {
        let path = self.home.daemon_state_path();
        if !path.exists() {
            return Ok(None);
        }

        let content = fs::read_to_string(path)?;
        let state = serde_json::from_str(&content).map_err(|err| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("machine daemon state is invalid JSON: {err}"),
            )
        })?;
        Ok(Some(state))
    }

    pub fn save(&self, state: &MachineDaemonState) -> Result<(), io::Error> {
        self.home.ensure_root()?;
        let content = serde_json::to_string_pretty(state).map_err(|err| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("failed to encode machine daemon state: {err}"),
            )
        })?;
        fs::write(self.home.daemon_state_path(), content)
    }

    pub fn write_running(&self, pid: u32) -> Result<MachineDaemonState, io::Error> {
        let now = now_ms();
        let state = MachineDaemonState {
            pid,
            started_at_ms: now,
            heartbeat_at_ms: now,
        };
        self.save(&state)?;
        Ok(state)
    }

    pub fn heartbeat(&self, pid: u32) -> Result<Option<MachineDaemonState>, io::Error> {
        let Some(mut state) = self.load()? else {
            return Ok(None);
        };
        if state.pid != pid {
            return Ok(Some(state));
        }
        state.heartbeat_at_ms = now_ms();
        self.save(&state)?;
        Ok(Some(state))
    }

    pub fn clear(&self) -> Result<(), io::Error> {
        let path = self.home.daemon_state_path();
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }

    pub fn load_active(&self) -> Result<Option<MachineDaemonState>, io::Error> {
        match self.load()? {
            Some(state) if is_process_running(state.pid) => Ok(Some(state)),
            Some(_) => {
                self.clear()?;
                Ok(None)
            }
            None => Ok(None),
        }
    }
}

pub fn is_process_running(pid: u32) -> bool {
    #[cfg(unix)]
    {
        if pid == 0 || pid > i32::MAX as u32 {
            return false;
        }
        // SAFETY: kill with signal 0 does not send a signal; it only checks process existence.
        let result = unsafe { libc::kill(pid as i32, 0) };
        if result == 0 {
            return true;
        }
        matches!(
            io::Error::last_os_error().raw_os_error(),
            Some(code) if code == libc::EPERM
        )
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        true
    }
}

pub fn terminate_process(pid: u32) -> Result<(), io::Error> {
    #[cfg(unix)]
    {
        if pid == 0 || pid > i32::MAX as u32 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid daemon pid {pid}"),
            ));
        }
        // SAFETY: sending SIGTERM to an explicit pid is the standard process shutdown path.
        let result = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
        if result == 0 {
            return Ok(());
        }
        return Err(io::Error::last_os_error());
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "machine daemon stop is only supported on unix in this build",
        ))
    }
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::{FileMachineDaemonStateStore, MachineDaemonState, ProjectorHome};
    use crate::test_support::temp_projector_home;

    #[test]
    fn daemon_state_round_trips_and_clears() {
        let store = FileMachineDaemonStateStore::new(ProjectorHome::new(temp_projector_home(
            "daemon-state",
        )));

        let running = store.write_running(4242).expect("write running state");
        let loaded = store.load().expect("load state").expect("state exists");
        assert_eq!(loaded.pid, 4242);
        assert_eq!(loaded.started_at_ms, running.started_at_ms);

        let heartbeat = store
            .heartbeat(4242)
            .expect("heartbeat state")
            .expect("state still exists");
        assert!(heartbeat.heartbeat_at_ms >= loaded.heartbeat_at_ms);

        store.clear().expect("clear state");
        assert_eq!(store.load().expect("load after clear"), None);
    }

    #[test]
    fn load_active_clears_stale_state() {
        let store = FileMachineDaemonStateStore::new(ProjectorHome::new(temp_projector_home(
            "daemon-stale",
        )));
        store
            .save(&MachineDaemonState {
                pid: u32::MAX,
                started_at_ms: 1,
                heartbeat_at_ms: 1,
            })
            .expect("write stale state");

        assert_eq!(store.load_active().expect("load active"), None);
        assert_eq!(store.load().expect("load after stale cleanup"), None);
    }
}
