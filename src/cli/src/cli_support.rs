use std::env;
use std::error::Error;
use std::path::{Component, Path, PathBuf};
use std::process::{self, Command};
use std::time::{SystemTime, UNIX_EPOCH};

use projector_domain::{ProvenanceEventKind, SyncEntryKind};
use projector_runtime::{
    FileMachineSyncRegistryStore, FileRepoSyncConfigStore, ProjectorHome, discover_repo_root,
};

pub(crate) fn print_usage() {
    println!(
        "Usage: projector <sync <start|stop|status>|connect [--id NAME] [--server host:port] [--ssh user@host]|connect status|disconnect <id> [--yes]|deploy [--profile NAME] [--ssh user@host] [--server-addr host:port] [--remote-dir PATH] [--sqlite-path PATH] [--listen-addr host:port] [--yes]|add [--force] [--profile ID] <repo-relative-path>|get [--profile ID] [sync-entry-id] [repo-relative-path]|remove <repo-relative-path>|rm <repo-relative-path>|doctor|status|log|history [--cursor N]|compact <repo-relative-path> [--revisions N --frequency N|--inherit]|restore [--confirm] <repo-relative-path>|redact [--confirm] <exact-text> <repo-relative-path>|purge [--confirm] <repo-relative-path>>"
    );
}

pub(crate) fn print_version() {
    println!("projector {}", env!("CARGO_PKG_VERSION"));
}

pub(crate) fn repo_root() -> Result<PathBuf, Box<dyn Error>> {
    let cwd = env::current_dir()?;
    Ok(discover_repo_root(&cwd))
}

pub(crate) fn display_paths(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(",")
}

pub(crate) fn format_sync_entry_kind(kind: &SyncEntryKind) -> &'static str {
    match kind {
        SyncEntryKind::File => "file",
        SyncEntryKind::Directory => "directory",
    }
}

pub(crate) fn format_kind(kind: &ProvenanceEventKind) -> &'static str {
    match kind {
        ProvenanceEventKind::DocumentCreated => "document_created",
        ProvenanceEventKind::DocumentUpdated => "document_updated",
        ProvenanceEventKind::DocumentDeleted => "document_deleted",
        ProvenanceEventKind::DocumentMoved => "document_moved",
        ProvenanceEventKind::DocumentHistoryRedacted => "document_history_redacted",
        ProvenanceEventKind::DocumentHistoryPurged => "document_history_purged",
        ProvenanceEventKind::SyncBootstrapped => "sync_bootstrapped",
        ProvenanceEventKind::SyncReusedBinding => "sync_reused_binding",
        ProvenanceEventKind::SyncIssue => "sync_issue",
        ProvenanceEventKind::SyncRecovery => "sync_recovery",
    }
}

pub(crate) fn make_id(prefix: &str) -> String {
    format!("{prefix}-{}", now_ns())
}

pub(crate) fn now_ns() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_nanos()
}

pub(crate) fn normalize_projection_relative_path(raw: &str) -> Result<PathBuf, Box<dyn Error>> {
    let path = Path::new(raw);
    if path.as_os_str().is_empty() {
        return Err("projection path must not be empty".into());
    }
    if path.is_absolute() {
        return Err("projection path must be repo-relative, not absolute".into());
    }

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err("projection path must not escape the repo root".into());
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err("projection path must be repo-relative".into());
            }
        }
    }

    if normalized.as_os_str().is_empty() {
        return Err("projection path must contain at least one normal path segment".into());
    }
    if normalized == Path::new(".projector") || normalized.starts_with(".projector") {
        return Err("projection path must not live inside .projector".into());
    }

    Ok(normalized)
}

pub(crate) fn sync_entry_id(path: &Path) -> String {
    let mut id = String::from("entry");
    for component in path.components() {
        if let Component::Normal(part) = component {
            id.push('-');
            id.push_str(&part.to_string_lossy());
        }
    }
    id
}

pub(crate) fn infer_sync_entry_kind(
    repo_root: &Path,
    normalized_path: &Path,
    raw_path: &str,
) -> SyncEntryKind {
    let absolute_path = repo_root.join(normalized_path);
    if absolute_path.is_file() {
        return SyncEntryKind::File;
    }
    if absolute_path.is_dir() {
        return SyncEntryKind::Directory;
    }
    if raw_path.ends_with('/') {
        return SyncEntryKind::Directory;
    }
    if normalized_path.extension().is_some() {
        return SyncEntryKind::File;
    }
    SyncEntryKind::Directory
}

pub(crate) fn is_path_tracked_by_git(
    repo_root: &Path,
    projection_relative_path: &Path,
) -> Result<bool, Box<dyn Error>> {
    let status = Command::new("git")
        .args(["ls-files", "--error-unmatch", "--"])
        .arg(projection_relative_path)
        .current_dir(repo_root)
        .stderr(process::Stdio::null())
        .status()?;
    Ok(status.success())
}

pub(crate) fn sync_machine_repo_registration(repo_root: &Path) -> Result<(), Box<dyn Error>> {
    let home = ProjectorHome::discover()?;
    let registry_store = FileMachineSyncRegistryStore::new(home);
    let config = FileRepoSyncConfigStore::new(repo_root).load()?;
    let _ = registry_store.sync_repo(repo_root, &config)?;
    Ok(())
}

pub(crate) fn ensure_gitignored(
    repo_root: &Path,
    projection_relative_path: &Path,
) -> Result<(), Box<dyn Error>> {
    if is_path_gitignored(repo_root, projection_relative_path)? {
        return Ok(());
    }

    Err(format!(
        "projection path {} is not gitignored",
        projection_relative_path.display()
    )
    .into())
}

pub(crate) fn is_path_gitignored(
    repo_root: &Path,
    projection_relative_path: &Path,
) -> Result<bool, Box<dyn Error>> {
    let candidates = [
        projection_relative_path.display().to_string(),
        format!("{}/", projection_relative_path.display()),
    ];

    for candidate in candidates {
        let status = Command::new("git")
            .arg("check-ignore")
            .arg("-q")
            .arg(&candidate)
            .current_dir(repo_root)
            .status()?;

        if status.success() {
            return Ok(true);
        }
    }

    Ok(false)
}
