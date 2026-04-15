/**
@module PROJECTOR.RUNTIME.SERVER_PROFILES
Owns the machine-global server profile registry used by connect and deploy flows to persist known remote authorities.
*/
// @fileimplements PROJECTOR.RUNTIME.SERVER_PROFILES
use std::fs;
use std::io;

use serde::{Deserialize, Serialize};

use super::ProjectorHome;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ServerProfileRegistry {
    pub profiles: Vec<ServerProfile>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ServerProfile {
    pub profile_id: String,
    pub server_addr: String,
    #[serde(default)]
    pub ssh_target: Option<String>,
}

#[derive(Clone, Debug)]
pub struct FileServerProfileStore {
    home: ProjectorHome,
}

impl FileServerProfileStore {
    pub fn new(home: ProjectorHome) -> Self {
        Self { home }
    }

    pub fn load(&self) -> Result<ServerProfileRegistry, io::Error> {
        let path = self.home.server_profiles_path();
        if !path.exists() {
            return Ok(ServerProfileRegistry::default());
        }

        let content = fs::read_to_string(path)?;
        serde_json::from_str(&content).map_err(|err| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("server profile registry is invalid JSON: {err}"),
            )
        })
    }

    pub fn save(&self, registry: &ServerProfileRegistry) -> Result<(), io::Error> {
        self.home.ensure_root()?;
        let content = serde_json::to_string_pretty(registry).map_err(|err| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("failed to encode server profile registry: {err}"),
            )
        })?;
        fs::write(self.home.server_profiles_path(), content)
    }

    pub fn upsert_profile(
        &self,
        profile_id: &str,
        server_addr: &str,
        ssh_target: Option<&str>,
    ) -> Result<ServerProfileRegistry, io::Error> {
        let mut registry = self.load()?;
        if let Some(profile) = registry
            .profiles
            .iter_mut()
            .find(|profile| profile.profile_id == profile_id)
        {
            profile.server_addr = server_addr.to_owned();
            profile.ssh_target = ssh_target.map(str::to_owned);
        } else {
            registry.profiles.push(ServerProfile {
                profile_id: profile_id.to_owned(),
                server_addr: server_addr.to_owned(),
                ssh_target: ssh_target.map(str::to_owned),
            });
        }
        registry
            .profiles
            .sort_by(|left, right| left.profile_id.cmp(&right.profile_id));
        self.save(&registry)?;
        Ok(registry)
    }

    pub fn remove_profile(&self, profile_id: &str) -> Result<ServerProfileRegistry, io::Error> {
        let mut registry = self.load()?;
        let original_len = registry.profiles.len();
        registry
            .profiles
            .retain(|profile| profile.profile_id != profile_id);
        if registry.profiles.len() == original_len {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("server profile {profile_id} is not registered"),
            ));
        }
        self.save(&registry)?;
        Ok(registry)
    }

    pub fn resolve_profile(&self, profile_id: &str) -> Result<Option<ServerProfile>, io::Error> {
        let registry = self.load()?;
        Ok(registry
            .profiles
            .into_iter()
            .find(|profile| profile.profile_id == profile_id))
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{FileServerProfileStore, ProjectorHome};

    fn temp_home(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("projector-home-{name}-{unique}"));
        std::fs::create_dir_all(&path).expect("create temp projector home");
        path
    }

    #[test]
    fn server_profile_store_round_trips_and_removes_profiles() {
        let store = FileServerProfileStore::new(ProjectorHome::new(temp_home("profiles")));
        store
            .upsert_profile("homebox", "127.0.0.1:7000", Some("spotless@host"))
            .expect("add profile");
        let registry = store
            .upsert_profile("workbox", "10.0.0.5:7001", None)
            .expect("add second profile");
        assert_eq!(registry.profiles.len(), 2);
        assert_eq!(
            registry.profiles[0].ssh_target.as_deref(),
            Some("spotless@host")
        );

        let registry = store.remove_profile("workbox").expect("remove profile");
        assert_eq!(registry.profiles.len(), 1);
        assert_eq!(registry.profiles[0].profile_id, "homebox");
        assert_eq!(registry.profiles[0].server_addr, "127.0.0.1:7000");
    }
}
