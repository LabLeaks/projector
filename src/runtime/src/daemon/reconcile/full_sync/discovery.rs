/**
@module PROJECTOR.RUNTIME.FULL_SYNC_DISCOVERY
Discovers local UTF-8 text files under configured projection mounts so full-sync move detection and lifecycle mutation logic can traverse the same repo-local file set.
*/
// @fileimplements PROJECTOR.RUNTIME.FULL_SYNC_DISCOVERY
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use projector_domain::SyncEntryKind;

pub(super) fn discover_local_text_files(
    root: &Path,
    kind: &SyncEntryKind,
) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let mut files = Vec::new();
    if !root.exists() {
        return Ok(files);
    }
    match kind {
        SyncEntryKind::Directory => collect_text_files(root, root, &mut files)?,
        SyncEntryKind::File => files.push(PathBuf::new()),
    }
    files.sort();
    Ok(files)
}

fn collect_text_files(
    root: &Path,
    current: &Path,
    files: &mut Vec<PathBuf>,
) -> Result<(), Box<dyn Error>> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_text_files(root, &path, files)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let relative_path = path
            .strip_prefix(root)
            .map_err(|err| format!("failed to relativize {}: {err}", path.display()))?;
        files.push(relative_path.to_path_buf());
    }
    Ok(())
}
