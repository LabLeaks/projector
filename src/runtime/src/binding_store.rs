/**
@module PROJECTOR.RUNTIME.BINDING
Persists checkout-local workspace binding and discovers repo roots for projector-managed state outside configured projection mounts.
*/
// @fileimplements PROJECTOR.RUNTIME.BINDING
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use projector_domain::{ActorId, CheckoutBinding, ProjectionRoots, SyncEntryKind, WorkspaceId};

pub trait BindingStore {
    type Error;

    fn load(&self) -> Result<Option<CheckoutBinding>, Self::Error>;
    fn save(&self, binding: &CheckoutBinding) -> Result<(), Self::Error>;
}

#[derive(Clone, Debug)]
pub struct FileBindingStore {
    repo_root: PathBuf,
}

impl FileBindingStore {
    pub fn new(repo_root: impl Into<PathBuf>) -> Self {
        Self {
            repo_root: repo_root.into(),
        }
    }

    fn projector_dir(&self) -> PathBuf {
        self.repo_root.join(".projector")
    }

    fn binding_path(&self) -> PathBuf {
        self.projector_dir().join("binding.txt")
    }
}

impl BindingStore for FileBindingStore {
    type Error = io::Error;

    fn load(&self) -> Result<Option<CheckoutBinding>, Self::Error> {
        let path = self.binding_path();
        if !path.exists() {
            return Ok(None);
        }

        let content = fs::read_to_string(path)?;
        let mut workspace_id = None;
        let mut actor_id = None;
        let mut projection_relative_paths = Vec::new();
        let mut server_addr = None;

        for line in content.lines() {
            if let Some(value) = line.strip_prefix("workspace_id=") {
                workspace_id = Some(WorkspaceId::new(value.to_owned()));
            } else if let Some(value) = line.strip_prefix("actor_id=") {
                actor_id = Some(ActorId::new(value.to_owned()));
            } else if let Some(value) = line.strip_prefix("projection_relative_path=") {
                projection_relative_paths.push(PathBuf::from(value));
            } else if let Some(value) = line.strip_prefix("server_addr=") {
                server_addr = Some(value.to_owned());
            }
        }

        match (
            workspace_id,
            actor_id,
            !projection_relative_paths.is_empty(),
        ) {
            (Some(workspace_id), Some(actor_id), true) => Ok(Some(CheckoutBinding {
                workspace_id,
                actor_id,
                roots: ProjectionRoots {
                    projector_dir: self.projector_dir(),
                    projection_paths: projection_relative_paths
                        .iter()
                        .map(|path| self.repo_root.join(path))
                        .collect(),
                },
                projection_kinds: vec![SyncEntryKind::Directory; projection_relative_paths.len()],
                projection_relative_paths,
                server_addr,
            })),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "binding file is missing required keys",
            )),
        }
    }

    fn save(&self, binding: &CheckoutBinding) -> Result<(), Self::Error> {
        fs::create_dir_all(self.projector_dir())?;
        let mut content = format!(
            "workspace_id={}\nactor_id={}\n",
            binding.workspace_id.as_str(),
            binding.actor_id.as_str(),
        );
        if let Some(server_addr) = &binding.server_addr {
            content.push_str(&format!("server_addr={server_addr}\n"));
        }
        for path in &binding.projection_relative_paths {
            content.push_str(&format!("projection_relative_path={}\n", path.display()));
        }
        fs::write(self.binding_path(), content)
    }
}

pub fn discover_repo_root(start: &Path) -> PathBuf {
    for candidate in start.ancestors() {
        if candidate.join(".jj").exists() || candidate.join(".git").exists() {
            return candidate.to_path_buf();
        }
    }
    start.to_path_buf()
}
