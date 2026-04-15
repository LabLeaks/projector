/**
@module PROJECTOR.RUNTIME.PROJECTOR_HOME
Owns discovery and path layout for the machine-global projector home that stores shared profile, repo-registry, and daemon-state files.
*/
// @fileimplements PROJECTOR.RUNTIME.PROJECTOR_HOME
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectorHome {
    root: PathBuf,
}

impl ProjectorHome {
    pub fn discover() -> Result<Self, io::Error> {
        if let Ok(path) = env::var("PROJECTOR_HOME") {
            return Ok(Self {
                root: PathBuf::from(path),
            });
        }

        let home = env::var("HOME").map_err(|_| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "projector home is not configured; set PROJECTOR_HOME or HOME",
            )
        })?;
        Ok(Self {
            root: PathBuf::from(home).join(".projector"),
        })
    }

    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn ensure_root(&self) -> Result<(), io::Error> {
        fs::create_dir_all(&self.root)
    }

    pub(crate) fn daemon_state_path(&self) -> PathBuf {
        self.root.join("daemon-state.json")
    }

    pub(crate) fn repo_registry_path(&self) -> PathBuf {
        self.root.join("repos.json")
    }

    pub(crate) fn server_profiles_path(&self) -> PathBuf {
        self.root.join("server-profiles.json")
    }
}
