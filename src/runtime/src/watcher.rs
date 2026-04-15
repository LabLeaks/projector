/**
@module PROJECTOR.RUNTIME.WATCHER
Observes projection mounts through notify-plus-polling and normalizes local file lifecycle events for the sync coordinator.
*/
// @fileimplements PROJECTOR.RUNTIME.WATCHER
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::SystemTime;

use notify::Watcher as NotifyWatcherTrait;
use notify::event::EventKind;
use notify::{RecommendedWatcher, RecursiveMode};
use projector_domain::SyncEntryKind;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WatcherEvent {
    FileChanged(PathBuf),
    FileCreated(PathBuf),
    FileDeleted(PathBuf),
}

pub trait Watcher {
    type Error;

    fn poll(&mut self) -> Result<Vec<WatcherEvent>, Self::Error>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WatchedMount {
    pub absolute_path: PathBuf,
    pub kind: SyncEntryKind,
}

pub struct RuntimeWatcher {
    notify: Option<NotifyWatcher>,
    polling: PollingWatcher,
}

impl RuntimeWatcher {
    pub fn new(mounts: Vec<WatchedMount>) -> Result<Self, io::Error> {
        let notify = NotifyWatcher::new(mounts.clone()).ok();
        let polling = PollingWatcher::new(mounts)?;
        Ok(Self { notify, polling })
    }

    pub fn poll_with_backstop(
        &mut self,
        run_polling_backstop: bool,
    ) -> Result<Vec<WatcherEvent>, io::Error> {
        let mut events = if run_polling_backstop || self.notify.is_none() {
            self.polling.poll()?
        } else {
            Vec::new()
        };
        if let Some(watcher) = &mut self.notify {
            events.extend(watcher.poll()?);
        }
        events.sort_by(|left, right| format!("{left:?}").cmp(&format!("{right:?}")));
        events.dedup();
        Ok(events)
    }
}

impl Watcher for RuntimeWatcher {
    type Error = io::Error;

    fn poll(&mut self) -> Result<Vec<WatcherEvent>, Self::Error> {
        self.poll_with_backstop(true)
    }
}

pub struct NotifyWatcher {
    _watcher: RecommendedWatcher,
    rx: Receiver<notify::Result<notify::Event>>,
}

impl NotifyWatcher {
    pub fn new(mounts: Vec<WatchedMount>) -> Result<Self, io::Error> {
        let (tx, rx) = mpsc::channel();
        let mut watcher = notify::recommended_watcher(move |result| {
            let _ = tx.send(result);
        })
        .map_err(io::Error::other)?;

        for mount in &mounts {
            watcher
                .watch(
                    &mount.absolute_path,
                    match mount.kind {
                        SyncEntryKind::Directory => RecursiveMode::Recursive,
                        SyncEntryKind::File => RecursiveMode::NonRecursive,
                    },
                )
                .map_err(io::Error::other)?;
        }

        Ok(Self {
            _watcher: watcher,
            rx,
        })
    }
}

impl Watcher for NotifyWatcher {
    type Error = io::Error;

    fn poll(&mut self) -> Result<Vec<WatcherEvent>, Self::Error> {
        let mut events = Vec::new();
        while let Ok(result) = self.rx.try_recv() {
            let event = result.map_err(io::Error::other)?;
            collect_notify_events(&mut events, event);
        }
        events.sort_by(|left, right| format!("{left:?}").cmp(&format!("{right:?}")));
        events.dedup();
        Ok(events)
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct WatchedPath {
    mount_root: PathBuf,
    relative_path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FileFingerprint {
    modified_at: SystemTime,
    len: u64,
}

#[derive(Clone, Debug)]
pub struct PollingWatcher {
    mounts: Vec<WatchedMount>,
    previous: BTreeMap<WatchedPath, FileFingerprint>,
}

impl PollingWatcher {
    pub fn new(mounts: Vec<WatchedMount>) -> Result<Self, io::Error> {
        let previous = scan_mounts(&mounts)?;
        Ok(Self { mounts, previous })
    }
}

impl Watcher for PollingWatcher {
    type Error = io::Error;

    fn poll(&mut self) -> Result<Vec<WatcherEvent>, Self::Error> {
        let current = scan_mounts(&self.mounts)?;
        let mut events = Vec::new();

        for (path, fingerprint) in &current {
            match self.previous.get(path) {
                None => events.push(WatcherEvent::FileCreated(join_relative_path(
                    &path.mount_root,
                    &path.relative_path,
                ))),
                Some(previous) if previous != fingerprint => {
                    events.push(WatcherEvent::FileChanged(join_relative_path(
                        &path.mount_root,
                        &path.relative_path,
                    )))
                }
                Some(_) => {}
            }
        }

        for path in self.previous.keys() {
            if !current.contains_key(path) {
                events.push(WatcherEvent::FileDeleted(join_relative_path(
                    &path.mount_root,
                    &path.relative_path,
                )));
            }
        }

        events.sort_by(|left, right| format!("{left:?}").cmp(&format!("{right:?}")));
        self.previous = current;
        Ok(events)
    }
}

fn scan_mounts(
    mounts: &[WatchedMount],
) -> Result<BTreeMap<WatchedPath, FileFingerprint>, io::Error> {
    let mut snapshot = BTreeMap::new();
    for mount in mounts {
        match mount.kind {
            SyncEntryKind::Directory => {
                scan_directory_mount(&mount.absolute_path, &mount.absolute_path, &mut snapshot)?;
            }
            SyncEntryKind::File => {
                scan_file_mount(&mount.absolute_path, &mut snapshot)?;
            }
        }
    }
    Ok(snapshot)
}

fn scan_directory_mount(
    mount_root: &Path,
    current: &Path,
    snapshot: &mut BTreeMap<WatchedPath, FileFingerprint>,
) -> Result<(), io::Error> {
    if !current.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            scan_directory_mount(mount_root, &path, snapshot)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }

        let metadata = entry.metadata()?;
        let modified_at = metadata.modified()?;
        let relative_path = path
            .strip_prefix(mount_root)
            .map_err(|err| io::Error::other(err.to_string()))?
            .to_path_buf();
        snapshot.insert(
            WatchedPath {
                mount_root: mount_root.to_path_buf(),
                relative_path,
            },
            FileFingerprint {
                modified_at,
                len: metadata.len(),
            },
        );
    }

    Ok(())
}

fn scan_file_mount(
    mount_path: &Path,
    snapshot: &mut BTreeMap<WatchedPath, FileFingerprint>,
) -> Result<(), io::Error> {
    if !mount_path.exists() || !mount_path.is_file() {
        return Ok(());
    }
    let metadata = fs::metadata(mount_path)?;
    snapshot.insert(
        WatchedPath {
            mount_root: mount_path.to_path_buf(),
            relative_path: PathBuf::new(),
        },
        FileFingerprint {
            modified_at: metadata.modified()?,
            len: metadata.len(),
        },
    );
    Ok(())
}

fn join_relative_path(root: &Path, relative_path: &Path) -> PathBuf {
    root.join(relative_path)
}

fn collect_notify_events(events: &mut Vec<WatcherEvent>, event: notify::Event) {
    let kind = event.kind;
    for path in event.paths {
        match kind {
            EventKind::Create(_) => events.push(WatcherEvent::FileCreated(path)),
            EventKind::Modify(_) => events.push(WatcherEvent::FileChanged(path)),
            EventKind::Remove(_) => {
                events.push(WatcherEvent::FileDeleted(path));
            }
            _ => {}
        }
    }
}
