/**
@module PROJECTOR.RUNTIME.MATERIALIZED_PATHS
Persists and loads the repo-local checkpoint of previously materialized live document paths under `.projector/` so full-sync reconciliation can distinguish remote-only files from local deletions.
*/
// @fileimplements PROJECTOR.RUNTIME.MATERIALIZED_PATHS
use std::collections::BTreeSet;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use projector_domain::BootstrapSnapshot;

fn materialized_paths_path(projector_dir: &Path) -> PathBuf {
    projector_dir.join("materialized_paths.txt")
}

pub(crate) fn load_materialized_paths(
    projector_dir: &Path,
) -> Result<BTreeSet<(PathBuf, PathBuf)>, Box<dyn Error>> {
    let path = materialized_paths_path(projector_dir);
    if !path.exists() {
        return Ok(BTreeSet::new());
    }

    let mut paths = BTreeSet::new();
    for line in fs::read_to_string(path)?.lines() {
        let mut parts = line.splitn(2, '\t');
        let mount = parts
            .next()
            .ok_or("invalid materialized path line: missing mount")?;
        let relative = parts
            .next()
            .ok_or("invalid materialized path line: missing relative path")?;
        paths.insert((PathBuf::from(mount), PathBuf::from(relative)));
    }
    Ok(paths)
}

pub(crate) fn save_materialized_paths(
    projector_dir: &Path,
    snapshot: &BootstrapSnapshot,
) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(projector_dir)?;
    let mut lines = snapshot
        .manifest
        .entries
        .iter()
        .filter(|entry| !entry.deleted)
        .map(|entry| {
            format!(
                "{}\t{}",
                entry.mount_relative_path.display(),
                entry.relative_path.display()
            )
        })
        .collect::<Vec<_>>();
    lines.sort();
    fs::write(materialized_paths_path(projector_dir), lines.join("\n"))?;
    Ok(())
}
