/**
@module PROJECTOR.EDGE.CONFLICT_SCAN
Owns local projected-text conflict marker scanning used by operational status reporting.
*/
// @fileimplements PROJECTOR.EDGE.CONFLICT_SCAN
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use projector_domain::ProjectionMount;

pub(super) fn find_conflicted_text_paths(
    repo_root: &Path,
    projection_mounts: &[ProjectionMount],
) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let mut conflicted = Vec::new();
    for mount in projection_mounts {
        collect_conflicted_text_paths(repo_root, mount, &mut conflicted)?;
    }
    conflicted.sort();
    Ok(conflicted)
}

fn collect_conflicted_text_paths(
    repo_root: &Path,
    mount: &ProjectionMount,
    conflicted: &mut Vec<PathBuf>,
) -> Result<(), Box<dyn Error>> {
    let path = &mount.absolute_path;
    if !path.exists() {
        return Ok(());
    }
    if mount.kind == projector_domain::SyncEntryKind::File {
        let Ok(text) = fs::read_to_string(path) else {
            return Ok(());
        };
        if text.contains("<<<<<<< existing") && text.contains(">>>>>>> incoming") {
            let relative = path.strip_prefix(repo_root).unwrap_or(path).to_path_buf();
            conflicted.push(relative);
        }
        return Ok(());
    }
    for child in walk_utf8_text_files(path)? {
        let Ok(text) = fs::read_to_string(&child) else {
            continue;
        };
        if text.contains("<<<<<<< existing") && text.contains(">>>>>>> incoming") {
            let relative = child
                .strip_prefix(repo_root)
                .unwrap_or(&child)
                .to_path_buf();
            conflicted.push(relative);
        }
    }
    Ok(())
}

fn walk_utf8_text_files(root: &Path) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let mut stack = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(path) = stack.pop() {
        if path.is_dir() {
            for entry in fs::read_dir(&path)? {
                stack.push(entry?.path());
            }
        } else {
            files.push(path);
        }
    }
    Ok(files)
}
