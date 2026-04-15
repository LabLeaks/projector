/**
@module PROJECTOR.RUNTIME.SYNC_ISSUES
Classifies runtime and transport failures into retry, rebootstrap, or manual-resolution buckets for bounded self-healing.
*/
// @fileimplements PROJECTOR.RUNTIME.SYNC_ISSUES
use std::error::Error;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SyncIssueDisposition {
    RetryImmediately,
    NeedsRebootstrap,
    ManualResolution,
}

impl SyncIssueDisposition {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::RetryImmediately => "retry_immediately",
            Self::NeedsRebootstrap => "needs_rebootstrap",
            Self::ManualResolution => "manual_resolution",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "retry_immediately" => Some(Self::RetryImmediately),
            "needs_rebootstrap" => Some(Self::NeedsRebootstrap),
            "manual_resolution" => Some(Self::ManualResolution),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyncIssue {
    pub code: String,
    pub disposition: SyncIssueDisposition,
    pub message: String,
}

pub fn classify_sync_issue(err: &dyn Error) -> SyncIssue {
    let message = err.to_string();

    if let Some(code) = extract_server_error_code(&message) {
        let disposition = match code.as_str() {
            "stale_cursor" | "document_deleted" | "document_moved" => {
                SyncIssueDisposition::NeedsRebootstrap
            }
            "path_taken" | "mount_mismatch" => SyncIssueDisposition::ManualResolution,
            _ => SyncIssueDisposition::ManualResolution,
        };
        return SyncIssue {
            code,
            disposition,
            message,
        };
    }

    let lower = message.to_ascii_lowercase();
    let disposition = if [
        "connection refused",
        "connection reset",
        "timed out",
        "broken pipe",
        "dns error",
        "temporary failure",
        "connection aborted",
    ]
    .iter()
    .any(|fragment| lower.contains(fragment))
    {
        SyncIssueDisposition::RetryImmediately
    } else {
        SyncIssueDisposition::ManualResolution
    };

    let code = match disposition {
        SyncIssueDisposition::RetryImmediately => "transport_unavailable",
        SyncIssueDisposition::NeedsRebootstrap => "stale_state",
        SyncIssueDisposition::ManualResolution => "unknown_sync_issue",
    }
    .to_owned();

    SyncIssue {
        code,
        disposition,
        message,
    }
}

fn extract_server_error_code(message: &str) -> Option<String> {
    let (_, tail) = message.split_once(": ")?;
    let (code, _) = tail.split_once(": ")?;
    if code
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch == '_' || ch.is_ascii_digit())
    {
        Some(code.to_owned())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::{SyncIssueDisposition, classify_sync_issue};

    #[test]
    fn classifies_stale_cursor_as_rebootstrap() {
        let err = io::Error::other(
            "create document request failed with status 409 Conflict: stale_cursor: manifest write based on stale cursor 0; current workspace cursor is 1",
        );
        let issue = classify_sync_issue(&err);
        assert_eq!(issue.code, "stale_cursor");
        assert_eq!(issue.disposition, SyncIssueDisposition::NeedsRebootstrap);
    }

    #[test]
    fn classifies_path_taken_as_manual_resolution() {
        let err = io::Error::other(
            "move document request failed with status 409 Conflict: path_taken: document already exists at private/index.html",
        );
        let issue = classify_sync_issue(&err);
        assert_eq!(issue.code, "path_taken");
        assert_eq!(issue.disposition, SyncIssueDisposition::ManualResolution);
    }

    #[test]
    fn classifies_transport_failures_as_retry_immediately() {
        let err = io::Error::other("tcp connect error: connection refused");
        let issue = classify_sync_issue(&err);
        assert_eq!(issue.code, "transport_unavailable");
        assert_eq!(issue.disposition, SyncIssueDisposition::RetryImmediately);
    }
}
