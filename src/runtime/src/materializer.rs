/**
@module PROJECTOR.RUNTIME.MATERIALIZER
Plans and applies filesystem projection mutations from manifest and body snapshot state onto configured gitignored mounts.
*/
// @fileimplements PROJECTOR.RUNTIME.MATERIALIZER
use std::collections::HashMap;
use std::path::PathBuf;
use std::{fs, io};

use projector_domain::{BootstrapSnapshot, DocumentId, SyncContext, SyncEntryKind};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MaterializationPlan {
    pub directories_to_create: Vec<PathBuf>,
    pub files_to_remove: Vec<PathBuf>,
    pub body_writes: Vec<(DocumentId, PathBuf, String)>,
}

pub trait Materializer {
    type Error;

    fn plan(&self, snapshot: &BootstrapSnapshot) -> Result<MaterializationPlan, Self::Error>;
    fn apply(&self, plan: &MaterializationPlan) -> Result<(), Self::Error>;
}

#[derive(Clone, Debug)]
struct MountPoint {
    relative_path: PathBuf,
    absolute_path: PathBuf,
    kind: SyncEntryKind,
}

#[derive(Clone, Debug)]
pub struct ProjectionMaterializer {
    mount_points: Vec<MountPoint>,
}

impl ProjectionMaterializer {
    pub fn new(binding: &dyn SyncContext) -> Self {
        Self {
            mount_points: binding
                .projection_mounts()
                .into_iter()
                .map(|mount| MountPoint {
                    relative_path: mount.relative_path,
                    absolute_path: mount.absolute_path,
                    kind: mount.kind,
                })
                .collect(),
        }
    }

    pub fn ensure_projection_roots(&self) -> Result<(), io::Error> {
        for mount in &self.mount_points {
            match mount.kind {
                SyncEntryKind::Directory => fs::create_dir_all(&mount.absolute_path)?,
                SyncEntryKind::File => {
                    if let Some(parent) = mount.absolute_path.parent() {
                        fs::create_dir_all(parent)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn resolve_target(
        &self,
        mount_relative_path: &PathBuf,
        relative_path: &PathBuf,
    ) -> Result<PathBuf, io::Error> {
        let mount = self
            .mount_points
            .iter()
            .find(|mount| mount.relative_path == *mount_relative_path)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "snapshot references unknown projection mount {}",
                        mount_relative_path.display()
                    ),
                )
            })?;
        match mount.kind {
            SyncEntryKind::Directory => Ok(mount.absolute_path.join(relative_path)),
            SyncEntryKind::File => {
                if relative_path.as_os_str().is_empty() {
                    Ok(mount.absolute_path.clone())
                } else {
                    Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "file projection mount {} cannot resolve nested path {}",
                            mount.relative_path.display(),
                            relative_path.display()
                        ),
                    ))
                }
            }
        }
    }

    pub fn resolve_projection_path(
        &self,
        mount_relative_path: &std::path::Path,
        relative_path: &std::path::Path,
    ) -> Result<PathBuf, io::Error> {
        self.resolve_target(
            &mount_relative_path.to_path_buf(),
            &relative_path.to_path_buf(),
        )
    }
}

impl Materializer for ProjectionMaterializer {
    type Error = io::Error;

    fn plan(&self, snapshot: &BootstrapSnapshot) -> Result<MaterializationPlan, Self::Error> {
        let body_by_id = snapshot
            .bodies
            .iter()
            .map(|body| (body.document_id.clone(), body.text.clone()))
            .collect::<HashMap<_, _>>();

        let mut plan = MaterializationPlan {
            directories_to_create: self
                .mount_points
                .iter()
                .filter_map(|mount| match mount.kind {
                    SyncEntryKind::Directory => Some(mount.absolute_path.clone()),
                    SyncEntryKind::File => mount.absolute_path.parent().map(|p| p.to_path_buf()),
                })
                .collect(),
            ..MaterializationPlan::default()
        };

        for entry in &snapshot.manifest.entries {
            let target = self.resolve_target(&entry.mount_relative_path, &entry.relative_path)?;
            if entry.deleted {
                plan.files_to_remove.push(target);
                continue;
            }

            let text = body_by_id.get(&entry.document_id).cloned().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "snapshot is missing body content for document {}",
                        entry.document_id.as_str()
                    ),
                )
            })?;

            if let Some(parent) = target.parent() {
                plan.directories_to_create.push(parent.to_path_buf());
            }
            plan.body_writes
                .push((entry.document_id.clone(), target, text));
        }

        plan.directories_to_create.sort();
        plan.directories_to_create.dedup();
        plan.files_to_remove.sort();
        plan.files_to_remove.dedup();
        Ok(plan)
    }

    fn apply(&self, plan: &MaterializationPlan) -> Result<(), Self::Error> {
        for dir in &plan.directories_to_create {
            fs::create_dir_all(dir)?;
        }
        for file in &plan.files_to_remove {
            if file.exists() {
                fs::remove_file(file)?;
            }
        }
        for (_, target, text) in &plan.body_writes {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(target, text)?;
        }
        Ok(())
    }
}
