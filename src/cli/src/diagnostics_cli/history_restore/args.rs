/**
@module PROJECTOR.EDGE.HISTORY_RESTORE_ARGS
Owns argument parsing and selector/defaulting rules for history and restore CLI flows.
*/
// @fileimplements PROJECTOR.EDGE.HISTORY_RESTORE_ARGS
use std::error::Error;
use std::io::IsTerminal;
use std::path::Path;

use projector_domain::DocumentBodyRevision;

pub(super) struct HistoryArgs {
    pub(super) mode: HistoryMode,
}

pub(super) enum HistoryMode {
    DocumentPath {
        repo_relative_path: String,
        limit: usize,
    },
    WorkspaceCursor {
        cursor: u64,
    },
}

pub(super) struct RestoreArgs {
    pub(super) repo_relative_path: String,
    pub(super) selector: Option<RestoreSelector>,
    pub(super) confirm: bool,
}

pub(super) struct RedactArgs {
    pub(super) exact_text: String,
    pub(super) repo_relative_path: String,
    pub(super) confirm: bool,
}

pub(super) struct PurgeArgs {
    pub(super) repo_relative_path: String,
    pub(super) confirm: bool,
}

pub(super) enum RestoreSelector {
    Seq(u64),
    Previous,
}

pub(super) fn parse_history_args(args: &[String]) -> Result<HistoryArgs, Box<dyn Error>> {
    let mut limit = 20_usize;
    let mut cursor = None;
    let mut repo_relative_path = None;
    let mut idx = 0;
    while idx < args.len() {
        match args[idx].as_str() {
            "--limit" => {
                idx += 1;
                limit = args
                    .get(idx)
                    .ok_or("missing value after --limit")?
                    .parse::<usize>()?;
            }
            "--cursor" => {
                idx += 1;
                if cursor.is_some() {
                    return Err("history accepts only one --cursor value".into());
                }
                cursor = Some(
                    args.get(idx)
                        .ok_or("missing value after --cursor")?
                        .parse::<u64>()?,
                );
            }
            other => {
                if repo_relative_path.is_some() {
                    return Err(format!("unexpected extra history argument: {other}").into());
                }
                repo_relative_path = Some(other.to_owned());
            }
        }
        idx += 1;
    }

    match (cursor, repo_relative_path) {
        (Some(cursor), None) => Ok(HistoryArgs {
            mode: HistoryMode::WorkspaceCursor { cursor },
        }),
        (None, Some(repo_relative_path)) => Ok(HistoryArgs {
            mode: HistoryMode::DocumentPath {
                repo_relative_path,
                limit,
            },
        }),
        (Some(_), Some(_)) => Err(
            "history accepts either --cursor <workspace-cursor> or a repo-relative path, not both"
                .into(),
        ),
        (None, None) => Err(
            "history requires either --cursor <workspace-cursor> or a repo-relative path argument"
                .into(),
        ),
    }
}

pub(super) fn parse_restore_args(args: &[String]) -> Result<RestoreArgs, Box<dyn Error>> {
    let mut repo_relative_path = None;
    let mut selector = None;
    let mut confirm = false;
    let mut idx = 0;
    while idx < args.len() {
        match args[idx].as_str() {
            "--confirm" => confirm = true,
            "--seq" => {
                idx += 1;
                if matches!(selector, Some(RestoreSelector::Seq(_))) {
                    return Err("restore accepts only one --seq value".into());
                }
                selector = Some(RestoreSelector::Seq(
                    args.get(idx)
                        .ok_or("missing value after --seq")?
                        .parse::<u64>()?,
                ));
            }
            "--previous" => {
                selector = Some(RestoreSelector::Previous);
            }
            other => {
                if repo_relative_path.is_some() {
                    return Err(format!("unexpected extra restore argument: {other}").into());
                }
                repo_relative_path = Some(other.to_owned());
            }
        }
        idx += 1;
    }

    Ok(RestoreArgs {
        repo_relative_path: repo_relative_path
            .ok_or("restore requires a repo-relative path argument")?,
        selector,
        confirm,
    })
}

pub(super) fn parse_redact_args(args: &[String]) -> Result<RedactArgs, Box<dyn Error>> {
    let mut confirm = false;
    let mut positionals = Vec::new();
    for arg in args {
        match arg.as_str() {
            "--confirm" => confirm = true,
            other => positionals.push(other.to_owned()),
        }
    }
    let [exact_text, repo_relative_path] = positionals.as_slice() else {
        return Err("redact requires <exact-text> <repo-relative-path>".into());
    };
    if exact_text.is_empty() {
        return Err("redact exact text must not be empty".into());
    }
    Ok(RedactArgs {
        exact_text: exact_text.clone(),
        repo_relative_path: repo_relative_path.clone(),
        confirm,
    })
}

pub(super) fn parse_purge_args(args: &[String]) -> Result<PurgeArgs, Box<dyn Error>> {
    let mut confirm = false;
    let mut repo_relative_path = None;
    for arg in args {
        match arg.as_str() {
            "--confirm" => confirm = true,
            other => {
                if repo_relative_path.is_some() {
                    return Err(format!("unexpected extra purge argument: {other}").into());
                }
                repo_relative_path = Some(other.to_owned());
            }
        }
    }
    Ok(PurgeArgs {
        repo_relative_path: repo_relative_path
            .ok_or("purge requires a repo-relative path argument")?,
        confirm,
    })
}

pub(super) fn should_use_restore_browser(restore_args: &RestoreArgs) -> bool {
    restore_args.selector.is_none()
        && std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
}

pub(super) fn default_restore_seq(
    revisions: &[DocumentBodyRevision],
    requested_path: &Path,
) -> Result<u64, Box<dyn Error>> {
    if revisions.is_empty() {
        return Err(format!(
            "document at {} does not have any body revisions",
            requested_path.display()
        )
        .into());
    }

    if revisions.len() >= 2 {
        Ok(revisions[revisions.len() - 2].seq)
    } else {
        Ok(revisions[0].seq)
    }
}

pub(super) fn resolve_restore_seq(
    restore_args: &RestoreArgs,
    revisions: &[DocumentBodyRevision],
    requested_path: &Path,
) -> Result<u64, Box<dyn Error>> {
    match restore_args.selector {
        Some(RestoreSelector::Seq(seq)) => Ok(seq),
        Some(RestoreSelector::Previous) | None => default_restore_seq(revisions, requested_path),
    }
}
