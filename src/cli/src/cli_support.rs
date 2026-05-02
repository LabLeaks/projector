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
        "\
projector {version}

Usage:
  projector <command> [options]
  projector help [command]

Repo sync:
  start                      Start or resume syncing for this repo
  stop                       Pause syncing for this repo
  stop --all                 Stop the machine-global daemon for every repo
  status                     Show repo sync health and daemon state
  add [--force] [--profile ID] <path>
                             Add a repo-local file or directory as a sync entry
  get [--profile ID] [--list] [filters] [entry-id] [path]
                             Discover or materialize remote sync entries
  remove <path>              Remove one repo-local sync entry
  rm <path>                  Alias for remove

Connections:
  connect [--id NAME] [--server host:port] [--ssh user@host]
                             Add a machine-global server profile
  connect status             List server profiles and local attachments
  disconnect <id> [--yes]    Remove a server profile
  deploy [options]           Provision and register a self-hosted server

Inspection and history:
  doctor                     Check profiles, registry, daemon, and sync entries
  log                        Show local projector events
  history [--cursor N] <path>
  restore [--confirm] <path>
  compact <path> [--revisions N --frequency N | --inherit]
  redact [--confirm] <exact-text> <path>
  purge [--confirm] <path>

Run `projector help <command>` for command-specific usage.",
        version = env!("CARGO_PKG_VERSION")
    );
}

pub(crate) fn print_command_help(command: &str) -> bool {
    let Some(help) = command_help(command) else {
        return false;
    };
    println!("{help}");
    true
}

pub(crate) fn is_help_arg(arg: &str) -> bool {
    arg == "--help" || arg == "-h"
}

pub(crate) fn unknown_command_error(command: &str) -> String {
    let mut message = format!("unknown command: {command}");
    if let Some(suggestion) = command_suggestion(command) {
        message.push_str("\n\nDid you mean:\n  ");
        message.push_str(suggestion);
    }
    message.push_str("\n\nRun `projector --help` for available commands.");
    message
}

fn command_suggestion(command: &str) -> Option<&'static str> {
    match command {
        "sync" => Some("projector start\n  projector stop\n  projector status"),
        "sync-stop-all" => Some("projector stop --all"),
        "ls" | "list" => Some("projector get --list"),
        _ => None,
    }
}

fn command_help(command: &str) -> Option<&'static str> {
    match command {
        "start" => Some(
            "\
Usage:
  projector start

Start or resume syncing for the current repo. If the machine-global daemon is not
running, projector starts it.",
        ),
        "stop" => Some(
            "\
Usage:
  projector stop
  projector stop --all

Pause syncing for the current repo. Use `--all` only when you want to stop the
machine-global daemon for every repo.",
        ),
        "add" => Some(
            "\
Usage:
  projector add [--force] [--profile ID] <repo-relative-path>

Add a repo-local file or directory as a sync entry. The path must be gitignored unless
`--force` is provided.",
        ),
        "get" => Some(
            "\
Usage:
  projector get [--profile ID] [--list] [--source-repo TEXT] [--remote-path TEXT]
  projector get [--profile ID] <sync-entry-id> [repo-relative-path]

Discover remote sync entries or materialize one into the current repo.",
        ),
        "remove" | "rm" => Some(
            "\
Usage:
  projector remove <repo-relative-path>
  projector rm <repo-relative-path>

Remove one repo-local sync entry from projector configuration.",
        ),
        "connect" => Some(
            "\
Usage:
  projector connect [--id NAME] [--server host:port] [--ssh user@host]
  projector connect status

Add or inspect machine-global server profiles.",
        ),
        "disconnect" => Some(
            "\
Usage:
  projector disconnect <id> [--yes]

Remove a machine-global server profile after warning about affected local attachments.",
        ),
        "deploy" => Some(
            "\
Usage:
  projector deploy [--profile NAME] [--ssh user@host] [--server-addr host:port]
                   [--remote-dir PATH] [--sqlite-path PATH] [--listen-addr host:port] [--yes]

Provision and register a self-hosted projector server.",
        ),
        "status" => Some(
            "\
Usage:
  projector status

Show repo-local sync health, including daemon state, pending work, and conflicts.",
        ),
        "doctor" => Some(
            "\
Usage:
  projector doctor

Check server profiles, current-repo sync entries, machine registry state, daemon state,
and recent sync issues.",
        ),
        "log" => Some(
            "\
Usage:
  projector log

Show local projector events for the current repo.",
        ),
        "history" => Some(
            "\
Usage:
  projector history <repo-relative-path>
  projector history --cursor N

Show retained document history or reconstruct a workspace at an earlier cursor.",
        ),
        "restore" => Some(
            "\
Usage:
  projector restore [--seq N] [--confirm] <repo-relative-path>

Preview or apply a retained document restore.",
        ),
        "compact" => Some(
            "\
Usage:
  projector compact <repo-relative-path>
  projector compact <repo-relative-path> --revisions N --frequency N
  projector compact <repo-relative-path> --inherit

Inspect or set retained-history compaction policy for a synced path.",
        ),
        "redact" => Some(
            "\
Usage:
  projector redact [--confirm] <exact-text> <repo-relative-path>

Preview or apply an exact-text retained-history redaction.",
        ),
        "purge" => Some(
            "\
Usage:
  projector purge [--confirm] <repo-relative-path>

Preview or clear retained body content for one synced path.",
        ),
        _ => None,
    }
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
