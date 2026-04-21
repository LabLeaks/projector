/**
@module PROJECTOR.RUNTIME.MATERIALIZED_PATHS
Persists and loads the repo-local checkpoint of previously materialized live document paths under `.projector/` so full-sync reconciliation can distinguish remote-only files from local deletions.
*/
// @fileimplements PROJECTOR.RUNTIME.MATERIALIZED_PATHS
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use projector_domain::{BootstrapSnapshot, DocumentId};

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
        let mut parts = line.split('\t');
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
    current_mounts: &BTreeSet<PathBuf>,
) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(projector_dir)?;
    let path = materialized_paths_path(projector_dir);
    let mut stored = load_materialized_path_records(&path)?;
    let mut checkpoint_mounts = current_mounts.clone();
    checkpoint_mounts.extend(
        snapshot
            .manifest
            .entries
            .iter()
            .map(|entry| entry.mount_relative_path.clone()),
    );
    stored.retain(|(mount_relative_path, _), _| !checkpoint_mounts.contains(mount_relative_path));

    let body_text_by_id = snapshot
        .bodies
        .iter()
        .map(|body| (body.document_id.clone(), body.text.as_str()))
        .collect::<std::collections::HashMap<DocumentId, &str>>();
    for entry in snapshot
        .manifest
        .entries
        .iter()
        .filter(|entry| !entry.deleted)
    {
        let Some(body_text) = body_text_by_id.get(&entry.document_id) else {
            continue;
        };
        stored.insert(
            (
                entry.mount_relative_path.clone(),
                entry.relative_path.clone(),
            ),
            Some(text_fingerprint(body_text)),
        );
    }

    let mut lines = stored
        .into_iter()
        .map(|((mount_relative_path, relative_path), fingerprint)| {
            if let Some(fingerprint) = fingerprint {
                format!(
                    "{}\t{}\t{}",
                    mount_relative_path.display(),
                    relative_path.display(),
                    fingerprint
                )
            } else {
                format!(
                    "{}\t{}",
                    mount_relative_path.display(),
                    relative_path.display()
                )
            }
        })
        .collect::<Vec<_>>();
    lines.sort();
    fs::write(path, lines.join("\n"))?;
    Ok(())
}

fn load_materialized_path_records(
    path: &Path,
) -> Result<BTreeMap<(PathBuf, PathBuf), Option<String>>, Box<dyn Error>> {
    if !path.exists() {
        return Ok(BTreeMap::new());
    }

    let mut records = BTreeMap::new();
    for line in fs::read_to_string(path)?.lines() {
        let mut parts = line.split('\t');
        let mount = parts
            .next()
            .ok_or("invalid materialized path line: missing mount")?;
        let relative = parts
            .next()
            .ok_or("invalid materialized path line: missing relative path")?;
        records.insert(
            (PathBuf::from(mount), PathBuf::from(relative)),
            parts.next().map(str::to_owned),
        );
    }
    Ok(records)
}

fn text_fingerprint(text: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use projector_domain::{
        BootstrapSnapshot, DocumentBody, DocumentId, DocumentKind, ManifestEntry, ManifestState,
    };

    use super::{save_materialized_paths, text_fingerprint};

    // @verifies PROJECTOR.SYNC.ROOT_RENAME_PRESERVES_SYNC_ENTRY_BINDINGS
    #[test]
    fn saving_one_mount_preserves_other_mount_relocation_evidence() {
        let projector_dir = temp_dir("preserve-other-mounts");
        let materialized_paths = projector_dir.join("materialized_paths.txt");
        fs::write(
            &materialized_paths,
            format!("notes\ttodo.md\t{}\n", text_fingerprint("existing notes\n")),
        )
        .expect("write existing materialized paths");
        let snapshot = BootstrapSnapshot {
            manifest: ManifestState {
                entries: vec![ManifestEntry {
                    document_id: DocumentId::new("doc-private"),
                    mount_relative_path: PathBuf::from("private"),
                    relative_path: PathBuf::from("briefs/index.md"),
                    kind: DocumentKind::Text,
                    deleted: false,
                }],
            },
            bodies: vec![DocumentBody {
                document_id: DocumentId::new("doc-private"),
                text: "private briefing\n".to_owned(),
            }],
        };

        save_materialized_paths(
            &projector_dir,
            &snapshot,
            &BTreeSet::from([PathBuf::from("private")]),
        )
        .expect("save materialized paths");

        let saved = fs::read_to_string(&materialized_paths).expect("read materialized paths");
        assert!(saved.contains(&format!(
            "notes\ttodo.md\t{}",
            text_fingerprint("existing notes\n")
        )));
        assert!(saved.contains(&format!(
            "private\tbriefs/index.md\t{}",
            text_fingerprint("private briefing\n")
        )));
    }

    #[test]
    fn saving_empty_mount_clears_that_mount_and_preserves_others() {
        let projector_dir = temp_dir("clear-empty-mount");
        let materialized_paths = projector_dir.join("materialized_paths.txt");
        fs::write(
            &materialized_paths,
            format!(
                "notes\ttodo.md\t{}\nprivate\tbriefs/index.md\t{}\n",
                text_fingerprint("existing notes\n"),
                text_fingerprint("old private\n")
            ),
        )
        .expect("write existing materialized paths");

        save_materialized_paths(
            &projector_dir,
            &BootstrapSnapshot::default(),
            &BTreeSet::from([PathBuf::from("private")]),
        )
        .expect("save materialized paths");

        let saved = fs::read_to_string(&materialized_paths).expect("read materialized paths");
        assert!(saved.contains(&format!(
            "notes\ttodo.md\t{}",
            text_fingerprint("existing notes\n")
        )));
        assert!(!saved.contains("private\tbriefs/index.md"));
    }

    fn temp_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("projector-materialized-{name}-{unique}"));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create temp dir");
        path
    }
}
