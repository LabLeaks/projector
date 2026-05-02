/**
@module PROJECTOR.RUNTIME.MATERIALIZED_PATHS
Persists and loads the repo-local checkpoint of previously materialized live document paths under `.projector/` so full-sync reconciliation can distinguish remote-only files from local deletions.
*/
// @fileimplements PROJECTOR.RUNTIME.MATERIALIZED_PATHS
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs;
use std::io;
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
        let (mount, relative, _) = parse_materialized_path_line(line)?;
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
        let fingerprint = body_text_by_id
            .get(&entry.document_id)
            .map(|body_text| text_fingerprint(body_text));
        stored.insert(
            (
                entry.mount_relative_path.clone(),
                entry.relative_path.clone(),
            ),
            fingerprint,
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
        let (mount, relative, fingerprint) = parse_materialized_path_line(line)?;
        records.insert(
            (PathBuf::from(mount), PathBuf::from(relative)),
            fingerprint.map(str::to_owned),
        );
    }
    Ok(records)
}

fn parse_materialized_path_line(line: &str) -> Result<(&str, &str, Option<&str>), Box<dyn Error>> {
    let parts = line.split('\t').collect::<Vec<_>>();
    match parts.as_slice() {
        [mount, relative] => Ok((mount, relative, None)),
        [mount, relative, fingerprint] => Ok((mount, relative, Some(fingerprint))),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid materialized path line: expected 2 or 3 tab-separated fields",
        )
        .into()),
    }
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
    use std::io;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use projector_domain::{
        BootstrapSnapshot, DocumentBody, DocumentId, DocumentKind, ManifestEntry, ManifestState,
    };

    use super::{
        load_materialized_path_records, load_materialized_paths, save_materialized_paths,
        text_fingerprint,
    };

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

    #[test]
    fn save_materialized_paths_preserves_manifest_paths_without_body_payload() {
        let projector_dir = temp_dir("manifest-path-without-body");
        let materialized_paths = projector_dir.join("materialized_paths.txt");
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
            bodies: Vec::new(),
        };

        save_materialized_paths(
            &projector_dir,
            &snapshot,
            &BTreeSet::from([PathBuf::from("private")]),
        )
        .expect("save materialized paths");

        let saved = fs::read_to_string(&materialized_paths).expect("read materialized paths");
        assert_eq!(saved, "private\tbriefs/index.md");
    }

    #[test]
    fn materialized_path_loaders_reject_extra_tab_separated_fields() {
        let projector_dir = temp_dir("invalid-extra-columns");
        let materialized_paths = projector_dir.join("materialized_paths.txt");
        fs::write(
            &materialized_paths,
            "private\tbriefs/index.md\tfingerprint\textra\n",
        )
        .expect("write invalid materialized paths");

        let load_paths_err = load_materialized_paths(&projector_dir)
            .expect_err("load materialized paths rejects extra field");
        assert_eq!(
            load_paths_err
                .downcast_ref::<io::Error>()
                .expect("io error")
                .kind(),
            io::ErrorKind::InvalidData
        );

        let load_records_err = load_materialized_path_records(&materialized_paths)
            .expect_err("load records rejects extra field");
        assert_eq!(
            load_records_err
                .downcast_ref::<io::Error>()
                .expect("io error")
                .kind(),
            io::ErrorKind::InvalidData
        );
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
