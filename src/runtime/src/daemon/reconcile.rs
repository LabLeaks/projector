/**
@module PROJECTOR.RUNTIME.RECONCILER
Coordinates runtime reconciliation by delegating full-sync bootstrap reconciliation and watch-path mutation reconciliation to narrower runtime modules.
*/
// @fileimplements PROJECTOR.RUNTIME.RECONCILER
use std::collections::BTreeSet;
use std::error::Error;
use std::io;
use std::path::{Path, PathBuf};

use projector_domain::{BootstrapSnapshot, ProjectionMount, SyncContext, SyncEntryKind};

use super::SyncRunner;
use crate::{MaterializationPlan, Materializer, Transport, WatcherEvent};

mod full_sync;
mod watch_sync;

impl<C, T> SyncRunner<'_, C, T>
where
    C: SyncContext,
    T: Transport<Error = io::Error>,
{
    pub(super) fn reconcile_snapshot(
        &mut self,
    ) -> Result<(BootstrapSnapshot, u64), Box<dyn Error>> {
        let Some(mut transport) = self.transport.take() else {
            return Ok((BootstrapSnapshot::default(), 0));
        };
        let result = full_sync::reconcile_snapshot(self, &mut transport);
        self.transport = Some(transport);
        result
    }

    pub(super) fn apply_snapshot(
        &self,
        snapshot: &BootstrapSnapshot,
    ) -> Result<(), Box<dyn Error>> {
        let projection_mounts = self.binding.projection_mounts();
        let known_mounts = projection_mounts
            .iter()
            .map(|mount| mount.relative_path.clone())
            .collect::<std::collections::BTreeSet<_>>();
        let previous_paths =
            full_sync::load_materialized_paths(self.binding.projector_dir()).unwrap_or_default();
        let held_mounts =
            missing_directory_mounts_with_saved_paths(&projection_mounts, &previous_paths);
        let mut plan = self.materializer.plan(snapshot)?;
        suppress_held_mount_materialization(&projection_mounts, &held_mounts, &mut plan);
        let current_live_paths = snapshot
            .manifest
            .entries
            .iter()
            .filter(|entry| !entry.deleted)
            .map(|entry| {
                (
                    entry.mount_relative_path.clone(),
                    entry.relative_path.clone(),
                )
            })
            .collect::<std::collections::BTreeSet<_>>();
        for (mount_relative_path, relative_path) in previous_paths {
            if !known_mounts.contains(&mount_relative_path) {
                continue;
            }
            if held_mounts.contains(&mount_relative_path) {
                continue;
            }
            if current_live_paths.contains(&(mount_relative_path.clone(), relative_path.clone())) {
                continue;
            }
            plan.files_to_remove.push(
                self.materializer
                    .resolve_projection_path(&mount_relative_path, &relative_path)?,
            );
        }
        plan.files_to_remove.sort();
        plan.files_to_remove.dedup();
        self.materializer.apply(&plan)?;
        full_sync::save_materialized_paths(self.binding.projector_dir(), snapshot, &known_mounts)?;
        full_sync::save_materialized_body_texts(self.binding.projector_dir(), snapshot)?;
        Ok(())
    }

    pub(super) fn pull_remote_snapshot_if_changed(
        &mut self,
        current_cursor: u64,
        current_snapshot: &BootstrapSnapshot,
    ) -> Result<Option<(BootstrapSnapshot, u64)>, Box<dyn Error>> {
        let Some(transport) = self.transport.as_mut() else {
            return Ok(None);
        };

        let (delta_snapshot, cursor) = transport.changes_since(self.binding, current_cursor)?;
        if cursor == current_cursor {
            return Ok(None);
        }

        Ok(Some((
            full_sync::merge_snapshots(current_snapshot.clone(), delta_snapshot),
            cursor,
        )))
    }

    pub(super) fn push_watcher_events(
        &mut self,
        current_snapshot: &BootstrapSnapshot,
        current_cursor: u64,
        events: &[WatcherEvent],
    ) -> Result<Option<(BootstrapSnapshot, u64)>, Box<dyn Error>> {
        let Some(mut transport) = self.transport.take() else {
            return Ok(None);
        };
        let result = watch_sync::push_watcher_events(
            self,
            &mut transport,
            current_snapshot,
            current_cursor,
            events,
        );
        self.transport = Some(transport);
        result
    }
}

pub(super) fn save_snapshot_checkpoints(
    projector_dir: &std::path::Path,
    snapshot: &BootstrapSnapshot,
    current_mounts: &BTreeSet<PathBuf>,
) -> Result<(), Box<dyn Error>> {
    full_sync::save_materialized_paths(projector_dir, snapshot, current_mounts)?;
    full_sync::save_materialized_body_texts(projector_dir, snapshot)?;
    Ok(())
}

pub(super) fn load_saved_materialized_paths(
    projector_dir: &std::path::Path,
) -> std::collections::BTreeSet<(std::path::PathBuf, std::path::PathBuf)> {
    full_sync::load_materialized_paths(projector_dir).unwrap_or_default()
}

pub(super) fn missing_directory_mounts_with_saved_paths(
    projection_mounts: &[ProjectionMount],
    previous_paths: &BTreeSet<(PathBuf, PathBuf)>,
) -> BTreeSet<PathBuf> {
    projection_mounts
        .iter()
        .filter(|mount| {
            mount.kind == SyncEntryKind::Directory
                && !mount.absolute_path.exists()
                && previous_paths
                    .iter()
                    .any(|(mount_relative_path, _)| mount_relative_path == &mount.relative_path)
        })
        .map(|mount| mount.relative_path.clone())
        .collect()
}

pub(super) fn suppress_held_mount_materialization(
    projection_mounts: &[ProjectionMount],
    held_mounts: &BTreeSet<PathBuf>,
    plan: &mut MaterializationPlan,
) {
    let held_absolute_roots = projection_mounts
        .iter()
        .filter(|mount| held_mounts.contains(&mount.relative_path))
        .map(|mount| mount.absolute_path.as_path())
        .collect::<Vec<_>>();
    plan.directories_to_create
        .retain(|path| !is_under_any(path, &held_absolute_roots));
    plan.files_to_remove
        .retain(|path| !is_under_any(path, &held_absolute_roots));
    plan.body_writes
        .retain(|(_, path, _)| !is_under_any(path, &held_absolute_roots));
}

fn is_under_any(path: &Path, roots: &[&Path]) -> bool {
    roots.iter().any(|root| path.starts_with(root))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use projector_domain::{DocumentId, ProjectionMount, SyncEntryKind};

    use crate::MaterializationPlan;

    use super::{missing_directory_mounts_with_saved_paths, suppress_held_mount_materialization};

    // @verifies PROJECTOR.SYNC.ROOT_RENAME_PRESERVES_SYNC_ENTRY_BINDINGS
    #[test]
    fn saved_paths_hold_missing_directory_mount_for_relocation() {
        let root = temp_dir("missing-directory-mount");
        let mount = ProjectionMount {
            relative_path: PathBuf::from("private"),
            absolute_path: root.join("private"),
            kind: SyncEntryKind::Directory,
        };
        let previous_paths =
            BTreeSet::from([(PathBuf::from("private"), PathBuf::from("briefs/index.md"))]);

        assert_eq!(
            missing_directory_mounts_with_saved_paths(&[mount], &previous_paths),
            BTreeSet::from([PathBuf::from("private")])
        );
    }

    #[test]
    fn missing_directory_without_saved_paths_does_not_hold_sync() {
        let root = temp_dir("missing-directory-no-paths");
        let mount = ProjectionMount {
            relative_path: PathBuf::from("private"),
            absolute_path: root.join("private"),
            kind: SyncEntryKind::Directory,
        };

        assert!(missing_directory_mounts_with_saved_paths(&[mount], &BTreeSet::new()).is_empty());
    }

    #[test]
    fn missing_file_mount_with_saved_paths_does_not_hold_sync() {
        let root = temp_dir("missing-file-mount");
        let mount = ProjectionMount {
            relative_path: PathBuf::from("README.md"),
            absolute_path: root.join("README.md"),
            kind: SyncEntryKind::File,
        };
        let previous_paths = BTreeSet::from([(PathBuf::from("README.md"), PathBuf::new())]);

        assert!(missing_directory_mounts_with_saved_paths(&[mount], &previous_paths).is_empty());
    }

    #[test]
    fn held_directory_mount_suppresses_only_that_mount_materialization() {
        let root = temp_dir("held-directory-plan");
        let private_mount = ProjectionMount {
            relative_path: PathBuf::from("private"),
            absolute_path: root.join("private"),
            kind: SyncEntryKind::Directory,
        };
        let notes_mount = ProjectionMount {
            relative_path: PathBuf::from("notes"),
            absolute_path: root.join("notes"),
            kind: SyncEntryKind::Directory,
        };
        let mut plan = MaterializationPlan {
            directories_to_create: vec![
                root.join("private"),
                root.join("private/briefs"),
                root.join("notes"),
                root.join("notes/daily"),
            ],
            files_to_remove: vec![
                root.join("private/briefs/old.md"),
                root.join("notes/daily/old.md"),
            ],
            body_writes: vec![
                (
                    DocumentId::new("doc-private"),
                    root.join("private/briefs/new.md"),
                    "private\n".to_owned(),
                ),
                (
                    DocumentId::new("doc-notes"),
                    root.join("notes/daily/new.md"),
                    "notes\n".to_owned(),
                ),
            ],
        };

        suppress_held_mount_materialization(
            &[private_mount, notes_mount],
            &BTreeSet::from([PathBuf::from("private")]),
            &mut plan,
        );

        assert_eq!(
            plan.directories_to_create,
            vec![root.join("notes"), root.join("notes/daily")]
        );
        assert_eq!(plan.files_to_remove, vec![root.join("notes/daily/old.md")]);
        assert_eq!(plan.body_writes.len(), 1);
        assert_eq!(plan.body_writes[0].1, root.join("notes/daily/new.md"));
    }

    fn temp_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("projector-reconcile-{name}-{unique}"));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create temp dir");
        path
    }
}
