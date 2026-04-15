/**
@module PROJECTOR.RUNTIME.PROVENANCE
Persists local audit and backstop events for sync bootstrap, recovery, and issue reporting under `.projector/`.
*/
// @fileimplements PROJECTOR.RUNTIME.PROVENANCE
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;

use projector_domain::{ActorId, ProvenanceEvent, ProvenanceEventKind};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredEvent {
    pub timestamp_ms: u128,
    pub actor_id: ActorId,
    pub kind: ProvenanceEventKind,
    pub path: String,
    pub summary: String,
}

#[derive(Clone, Debug)]
pub struct FileProvenanceLog {
    path: PathBuf,
}

impl FileProvenanceLog {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn append(&self, event: &StoredEvent) -> Result<(), io::Error> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(
            file,
            "{}\t{}\t{}\t{}\t{}",
            event.timestamp_ms,
            event.actor_id.as_str(),
            format_kind(&event.kind),
            event.path.replace('\t', " "),
            event.summary.replace('\t', " ")
        )
    }

    pub fn read_all(&self) -> Result<Vec<StoredEvent>, io::Error> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let content = fs::read_to_string(&self.path)?;
        let mut events = Vec::new();

        for line in content.lines() {
            let mut parts = line.splitn(5, '\t');
            let timestamp_ms = parts
                .next()
                .and_then(|value| value.parse::<u128>().ok())
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "invalid event timestamp")
                })?;
            let actor_id = parts
                .next()
                .map(|value| ActorId::new(value.to_owned()))
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid event actor"))?;
            let kind = parts
                .next()
                .and_then(parse_kind)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid event kind"))?;
            let path = parts
                .next()
                .map(str::to_owned)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid event path"))?;
            let summary = parts.next().map(str::to_owned).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "invalid event summary")
            })?;
            events.push(StoredEvent {
                timestamp_ms,
                actor_id,
                kind,
                path,
                summary,
            });
        }

        Ok(events)
    }
}

fn format_kind(kind: &ProvenanceEventKind) -> &'static str {
    match kind {
        ProvenanceEventKind::SyncBootstrapped => "sync_bootstrapped",
        ProvenanceEventKind::SyncReusedBinding => "sync_reused_binding",
        ProvenanceEventKind::SyncRecovery => "sync_recovery",
        ProvenanceEventKind::SyncIssue => "sync_issue",
        ProvenanceEventKind::DocumentCreated => "document_created",
        ProvenanceEventKind::DocumentMoved => "document_moved",
        ProvenanceEventKind::DocumentUpdated => "document_updated",
        ProvenanceEventKind::DocumentDeleted => "document_deleted",
    }
}

fn parse_kind(value: &str) -> Option<ProvenanceEventKind> {
    match value {
        "sync_bootstrapped" => Some(ProvenanceEventKind::SyncBootstrapped),
        "sync_reused_binding" => Some(ProvenanceEventKind::SyncReusedBinding),
        "sync_recovery" => Some(ProvenanceEventKind::SyncRecovery),
        "sync_issue" => Some(ProvenanceEventKind::SyncIssue),
        "document_created" => Some(ProvenanceEventKind::DocumentCreated),
        "document_moved" => Some(ProvenanceEventKind::DocumentMoved),
        "document_updated" => Some(ProvenanceEventKind::DocumentUpdated),
        "document_deleted" => Some(ProvenanceEventKind::DocumentDeleted),
        _ => None,
    }
}

impl From<StoredEvent> for ProvenanceEvent {
    fn from(value: StoredEvent) -> Self {
        Self {
            cursor: 0,
            timestamp_ms: value.timestamp_ms,
            actor_id: value.actor_id,
            document_id: None,
            mount_relative_path: None,
            relative_path: Some(value.path),
            summary: value.summary,
            kind: value.kind,
        }
    }
}
