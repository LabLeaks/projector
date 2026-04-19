/**
@module PROJECTOR.TESTS.SYNC_BOOTSTRAP
Bootstrap, delta, and convergence proof under the local bootstrap harness.
*/
// @fileimplements PROJECTOR.TESTS.SYNC_BOOTSTRAP
use super::*;

// @verifies PROJECTOR.SERVER.SYNC.CHANGES_SINCE_RETURNS_CHANGED_DOCUMENTS
#[test]
fn changes_since_returns_only_changed_documents() {
    let repo = temp_repo("changes-since");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    let binding = load_workspace_binding_from_sync_config(&repo);
    let mut transport = HttpTransport::new(format!("http://{addr}"));

    let (initial_snapshot, initial_cursor) = transport.bootstrap(&binding).expect("bootstrap");
    assert!(initial_snapshot.manifest.entries.is_empty());
    assert_eq!(initial_cursor, 0);

    let document_id = transport
        .create_document(
            &binding,
            initial_cursor,
            Path::new("private"),
            Path::new("briefs/index.html"),
            "<p>delta create</p>\n",
        )
        .expect("create document");

    let (delta_snapshot, next_cursor) = transport
        .changes_since(&binding, initial_cursor)
        .expect("delta after create");
    assert!(next_cursor > initial_cursor);
    assert_eq!(delta_snapshot.manifest.entries.len(), 1);
    assert_eq!(delta_snapshot.bodies.len(), 1);
    assert_eq!(delta_snapshot.manifest.entries[0].document_id, document_id);
    assert_eq!(delta_snapshot.bodies[0].document_id, document_id);
    assert_eq!(delta_snapshot.bodies[0].text, "<p>delta create</p>\n");

    let (empty_delta, stable_cursor) = transport
        .changes_since(&binding, next_cursor)
        .expect("empty delta");
    assert!(empty_delta.manifest.entries.is_empty());
    assert!(empty_delta.bodies.is_empty());
    assert_eq!(stable_cursor, next_cursor);
}

// @verifies PROJECTOR.SERVER.DOCUMENTS.REJECTS_STALE_MANIFEST_WRITES
#[test]
fn stale_manifest_writes_return_conflict_errors() {
    let repo = temp_repo("stale-manifest");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    let binding = load_workspace_binding_from_sync_config(&repo);
    let mut transport = HttpTransport::new(format!("http://{addr}"));

    let (_snapshot, initial_cursor) = transport.bootstrap(&binding).expect("bootstrap");
    transport
        .create_document(
            &binding,
            initial_cursor,
            Path::new("private"),
            Path::new("briefs/current.html"),
            "<p>fresh write</p>\n",
        )
        .expect("initial manifest write");

    let err = transport
        .create_document(
            &binding,
            initial_cursor,
            Path::new("private"),
            Path::new("briefs/stale.html"),
            "<p>stale write</p>\n",
        )
        .expect_err("stale manifest write should fail");

    let message = err.to_string();
    assert!(
        message.contains("409 Conflict"),
        "unexpected error: {message}"
    );
    assert!(
        message.contains("stale_cursor"),
        "unexpected error: {message}"
    );
}

#[test]
fn sync_bootstraps_bound_workspace_against_server() {
    let repo = temp_repo("server-sync");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir);

    let addr = addr.to_string();
    let stdout = run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);

    let metadata = fs::read_to_string(
        state_dir
            .join("workspaces")
            .join(
                stdout
                    .lines()
                    .find_map(|line| line.strip_prefix("workspace_id: "))
                    .expect("workspace id"),
            )
            .join("metadata.txt"),
    )
    .expect("read server metadata");

    assert!(stdout.contains(&format!("server_addr: {addr}")));
    assert!(metadata.contains("projection_relative_path=private"));
    assert!(metadata.contains("projection_relative_path=notes"));
}

#[test]
fn sync_applies_initial_snapshot_from_server() {
    let repo = temp_repo("snapshot-sync");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir);

    let addr = addr.to_string();
    let first_sync = run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    let workspace_id = first_sync
        .lines()
        .find_map(|line| line.strip_prefix("workspace_id: "))
        .expect("workspace id")
        .to_owned();

    let snapshot = BootstrapSnapshot {
        manifest: ManifestState {
            entries: vec![ManifestEntry {
                document_id: DocumentId::new("doc-1"),
                mount_relative_path: PathBuf::from("private"),
                relative_path: PathBuf::from("briefs/today.md"),
                kind: DocumentKind::Text,
                deleted: false,
            }],
        },
        bodies: vec![DocumentBody {
            document_id: DocumentId::new("doc-1"),
            text: "# Today\n\nShip the bootstrap path.\n".to_owned(),
        }],
    };
    projector_server::write_workspace_snapshot(&state_dir, &workspace_id, &snapshot)
        .expect("write workspace snapshot");

    let second_sync = run_projector(&repo, &["sync"]);

    assert!(second_sync.contains("binding: reused"));
    assert_eq!(
        fs::read_to_string(repo.join("private/briefs/today.md")).expect("read materialized file"),
        "# Today\n\nShip the bootstrap path.\n"
    );
}

#[test]
fn sync_pushes_local_text_creations_to_server() {
    let repo = temp_repo("local-create-sync");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    fs::create_dir_all(repo.join("private/briefs")).expect("create local subdir");
    fs::write(
        repo.join("private/briefs/index.html"),
        "<h1>Local</h1>\n<p>Created before the next sync.</p>\n",
    )
    .expect("write local text file");

    let second_sync = run_projector(&repo, &["sync"]);
    let snapshot = fs::read_to_string(
        state_dir
            .join("workspaces")
            .read_dir()
            .expect("workspace dir")
            .next()
            .expect("workspace entry")
            .expect("workspace dir entry")
            .path()
            .join("snapshot.json"),
    )
    .expect("read server snapshot");
    let log = run_projector(&repo, &["log"]);

    assert!(second_sync.contains("binding: reused"));
    assert!(snapshot.contains("\"mount_relative_path\": \"private\""));
    assert!(snapshot.contains("\"relative_path\": \"briefs/index.html\""));
    assert!(snapshot.contains("Created before the next sync."));
    assert!(log.contains("kind=document_created"));
    assert!(log.contains("path=private/briefs/index.html"));
}

// @verifies PROJECTOR.WORKSPACE.TEXT_ONLY
#[test]
fn sync_ignores_non_utf8_files_under_projection_mounts() {
    let repo = temp_repo("text-only");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    fs::create_dir_all(repo.join("private/assets")).expect("create local subdir");
    fs::write(repo.join("private/assets/blob.bin"), [0, 159, 146, 150]).expect("write binary");
    fs::write(
        repo.join("private/assets/readme.txt"),
        "still text and should sync\n",
    )
    .expect("write text file");

    run_projector(&repo, &["sync"]);

    let binding = load_workspace_binding_from_sync_config(&repo);
    let mut transport = HttpTransport::new(format!("http://{addr}"));
    let (snapshot, _cursor) = transport.bootstrap(&binding).expect("bootstrap");

    assert!(snapshot.manifest.entries.iter().any(|entry| {
        !entry.deleted
            && entry.mount_relative_path == Path::new("private")
            && entry.relative_path == Path::new("assets/readme.txt")
    }));
    assert!(!snapshot.manifest.entries.iter().any(|entry| {
        !entry.deleted
            && entry.mount_relative_path == Path::new("private")
            && entry.relative_path == Path::new("assets/blob.bin")
    }));
}

#[test]
fn sync_pushes_local_text_updates_to_server() {
    let repo = temp_repo("local-update-sync");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    let first_sync = run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    let workspace_id = first_sync
        .lines()
        .find_map(|line| line.strip_prefix("workspace_id: "))
        .expect("workspace id")
        .to_owned();

    let snapshot = BootstrapSnapshot {
        manifest: ManifestState {
            entries: vec![ManifestEntry {
                document_id: DocumentId::new("doc-2"),
                mount_relative_path: PathBuf::from("private"),
                relative_path: PathBuf::from("briefs/index.html"),
                kind: DocumentKind::Text,
                deleted: false,
            }],
        },
        bodies: vec![DocumentBody {
            document_id: DocumentId::new("doc-2"),
            text: "<h1>Old</h1>\n<p>Before edit.</p>\n".to_owned(),
        }],
    };
    projector_server::write_workspace_snapshot(&state_dir, &workspace_id, &snapshot)
        .expect("write workspace snapshot");

    run_projector(&repo, &["sync"]);
    fs::write(
        repo.join("private/briefs/index.html"),
        "<h1>New</h1>\n<p>After edit.</p>\n",
    )
    .expect("write updated local text file");

    let third_sync = run_projector(&repo, &["sync"]);
    let updated_snapshot = fs::read_to_string(
        state_dir
            .join("workspaces")
            .join(&workspace_id)
            .join("snapshot.json"),
    )
    .expect("read server snapshot");
    let log = run_projector(&repo, &["log"]);

    assert!(third_sync.contains("binding: reused"));
    assert!(updated_snapshot.contains("<h1>New</h1>"));
    assert!(updated_snapshot.contains("After edit."));
    assert!(log.contains("kind=document_updated"));
    assert!(log.contains("path=private/briefs/index.html"));
}

#[test]
fn sync_pushes_local_text_deletions_to_server() {
    let repo = temp_repo("local-delete-sync");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    let first_sync = run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    let workspace_id = first_sync
        .lines()
        .find_map(|line| line.strip_prefix("workspace_id: "))
        .expect("workspace id")
        .to_owned();

    let snapshot = BootstrapSnapshot {
        manifest: ManifestState {
            entries: vec![ManifestEntry {
                document_id: DocumentId::new("doc-3"),
                mount_relative_path: PathBuf::from("private"),
                relative_path: PathBuf::from("briefs/index.html"),
                kind: DocumentKind::Text,
                deleted: false,
            }],
        },
        bodies: vec![DocumentBody {
            document_id: DocumentId::new("doc-3"),
            text: "<h1>Delete Me</h1>\n".to_owned(),
        }],
    };
    projector_server::write_workspace_snapshot(&state_dir, &workspace_id, &snapshot)
        .expect("write workspace snapshot");

    run_projector(&repo, &["sync"]);
    fs::remove_file(repo.join("private/briefs/index.html")).expect("remove local file");

    let third_sync = run_projector(&repo, &["sync"]);
    let updated_snapshot = fs::read_to_string(
        state_dir
            .join("workspaces")
            .join(&workspace_id)
            .join("snapshot.json"),
    )
    .expect("read server snapshot");
    let log = run_projector(&repo, &["log"]);

    assert!(third_sync.contains("binding: reused"));
    assert!(updated_snapshot.contains("\"deleted\": true"));
    assert!(log.contains("kind=document_deleted"));
    assert!(log.contains("path=private/briefs/index.html"));
}

#[test]
fn sync_pushes_local_text_moves_to_server() {
    let repo = temp_repo("local-move-sync");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    let first_sync = run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    let workspace_id = first_sync
        .lines()
        .find_map(|line| line.strip_prefix("workspace_id: "))
        .expect("workspace id")
        .to_owned();

    let snapshot = BootstrapSnapshot {
        manifest: ManifestState {
            entries: vec![ManifestEntry {
                document_id: DocumentId::new("doc-4"),
                mount_relative_path: PathBuf::from("private"),
                relative_path: PathBuf::from("briefs/index.html"),
                kind: DocumentKind::Text,
                deleted: false,
            }],
        },
        bodies: vec![DocumentBody {
            document_id: DocumentId::new("doc-4"),
            text: "<h1>Move Me</h1>\n".to_owned(),
        }],
    };
    projector_server::write_workspace_snapshot(&state_dir, &workspace_id, &snapshot)
        .expect("write workspace snapshot");

    run_projector(&repo, &["sync"]);
    fs::create_dir_all(repo.join("private/archive")).expect("create archive dir");
    fs::rename(
        repo.join("private/briefs/index.html"),
        repo.join("private/archive/index.html"),
    )
    .expect("rename local file");

    let third_sync = run_projector(&repo, &["sync"]);
    let updated_snapshot = fs::read_to_string(
        state_dir
            .join("workspaces")
            .join(&workspace_id)
            .join("snapshot.json"),
    )
    .expect("read server snapshot");
    let log = run_projector(&repo, &["log"]);

    assert!(third_sync.contains("binding: reused"));
    assert!(updated_snapshot.contains("\"document_id\": \"doc-4\""));
    assert!(updated_snapshot.contains("\"relative_path\": \"archive/index.html\""));
    assert!(!updated_snapshot.contains("\"relative_path\": \"briefs/index.html\",\n      \"kind\": \"Text\",\n      \"deleted\": false"));
    assert!(repo.join("private/archive/index.html").exists());
    assert!(!repo.join("private/briefs/index.html").exists());
    assert!(log.contains("kind=document_moved"));
    assert!(log.contains("path=private/archive/index.html"));
}

// @verifies PROJECTOR.SYNC.FILE_LIFECYCLE
#[test]
fn sync_reconciles_text_file_lifecycle_through_server_manifest() {
    let repo = temp_repo("file-lifecycle");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    fs::create_dir_all(repo.join("private/briefs")).expect("create local subdir");
    fs::write(
        repo.join("private/briefs/lifecycle.html"),
        "<p>created</p>\n",
    )
    .expect("write initial file");

    run_projector(&repo, &["sync"]);

    let binding = load_workspace_binding_from_sync_config(&repo);
    let mut transport = HttpTransport::new(format!("http://{addr}"));
    let (created_snapshot, created_cursor) =
        transport.bootstrap(&binding).expect("bootstrap create");
    let created_entry = created_snapshot
        .manifest
        .entries
        .iter()
        .find(|entry| {
            !entry.deleted
                && entry.mount_relative_path == Path::new("private")
                && entry.relative_path == Path::new("briefs/lifecycle.html")
        })
        .expect("created manifest entry");
    let created_document_id = created_entry.document_id.clone();
    assert!(
        created_snapshot
            .bodies
            .iter()
            .any(|body| body.document_id == created_document_id && body.text == "<p>created</p>\n")
    );

    fs::write(
        repo.join("private/briefs/lifecycle.html"),
        "<p>updated</p>\n",
    )
    .expect("update file");
    run_projector(&repo, &["sync"]);

    let (updated_snapshot, updated_cursor) = transport
        .changes_since(&binding, created_cursor)
        .expect("delta after update");
    assert!(updated_cursor > created_cursor);
    assert!(
        updated_snapshot
            .bodies
            .iter()
            .any(|body| body.document_id == created_document_id && body.text == "<p>updated</p>\n")
    );

    fs::create_dir_all(repo.join("notes/archive")).expect("create move target");
    fs::rename(
        repo.join("private/briefs/lifecycle.html"),
        repo.join("notes/archive/lifecycle.html"),
    )
    .expect("move file");
    run_projector(&repo, &["sync"]);

    let (moved_snapshot, moved_cursor) = transport
        .changes_since(&binding, updated_cursor)
        .expect("delta after move");
    let moved_entry = moved_snapshot
        .manifest
        .entries
        .iter()
        .find(|entry| entry.document_id == created_document_id)
        .expect("moved manifest entry");
    assert!(!moved_entry.deleted);
    assert_eq!(moved_entry.mount_relative_path, Path::new("notes"));
    assert_eq!(
        moved_entry.relative_path,
        Path::new("archive/lifecycle.html")
    );

    fs::remove_file(repo.join("notes/archive/lifecycle.html")).expect("delete moved file");
    run_projector(&repo, &["sync"]);

    let (deleted_snapshot, deleted_cursor) = transport
        .changes_since(&binding, moved_cursor)
        .expect("delta after delete");
    assert!(deleted_cursor > moved_cursor);
    let deleted_entry = deleted_snapshot
        .manifest
        .entries
        .iter()
        .find(|entry| entry.document_id == created_document_id)
        .expect("deleted manifest entry");
    assert!(deleted_entry.deleted);
    assert!(
        deleted_snapshot
            .bodies
            .iter()
            .all(|body| body.document_id != created_document_id)
    );
}

// @verifies PROJECTOR.PROVENANCE.EVENT_LOG
#[test]
fn server_provenance_records_append_only_file_lifecycle_events() {
    let repo = temp_repo("provenance-events");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    let state_dir = repo.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    fs::create_dir_all(repo.join("private/briefs")).expect("create local subdir");
    fs::write(
        repo.join("private/briefs/provenance.html"),
        "<p>create</p>\n",
    )
    .expect("write create");
    run_projector(&repo, &["sync"]);

    fs::write(
        repo.join("private/briefs/provenance.html"),
        "<p>update</p>\n",
    )
    .expect("write update");
    run_projector(&repo, &["sync"]);

    fs::create_dir_all(repo.join("notes/archive")).expect("create move target");
    fs::rename(
        repo.join("private/briefs/provenance.html"),
        repo.join("notes/archive/provenance.html"),
    )
    .expect("move file");
    run_projector(&repo, &["sync"]);

    fs::remove_file(repo.join("notes/archive/provenance.html")).expect("delete moved file");
    run_projector(&repo, &["sync"]);

    let binding = load_workspace_binding_from_sync_config(&repo);
    let mut transport = HttpTransport::new(format!("http://{addr}"));
    let events = transport.provenance(&binding, 20).expect("list provenance");

    assert!(events.iter().any(|event| {
        event.kind == projector_domain::ProvenanceEventKind::DocumentCreated
            && event.mount_relative_path.as_deref() == Some("private")
            && event.relative_path.as_deref() == Some("briefs/provenance.html")
            && !event.actor_id.as_str().is_empty()
            && event.timestamp_ms > 0
            && event.summary.contains("created")
    }));
    assert!(events.iter().any(|event| {
        event.kind == projector_domain::ProvenanceEventKind::DocumentUpdated
            && event.mount_relative_path.as_deref() == Some("private")
            && event.relative_path.as_deref() == Some("briefs/provenance.html")
            && event.summary.contains("updated")
    }));
    assert!(events.iter().any(|event| {
        event.kind == projector_domain::ProvenanceEventKind::DocumentMoved
            && event.mount_relative_path.as_deref() == Some("notes")
            && event.relative_path.as_deref() == Some("archive/provenance.html")
            && event.summary.contains("moved")
    }));
    assert!(events.iter().any(|event| {
        event.kind == projector_domain::ProvenanceEventKind::DocumentDeleted
            && event.summary.contains("deleted")
    }));
}

// @verifies PROJECTOR.SYNC.TEXT_CONVERGENCE
#[test]
fn sync_converges_concurrent_text_updates_across_two_bound_checkouts() {
    let repo_a = temp_repo("text-convergence-a");
    let repo_b = temp_repo("text-convergence-b");
    fs::write(repo_a.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    fs::write(repo_b.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");

    let state_dir = repo_a.join("server-state");
    let addr = spawn_server(&state_dir).to_string();

    run_projector(&repo_a, &["sync", "--server", &addr, "private", "notes"]);
    clone_sync_config_for_repo(&repo_a, &repo_b, "actor-text-convergence-b");

    fs::create_dir_all(repo_a.join("private/briefs")).expect("create base dir");
    fs::write(
        repo_a.join("private/briefs/converge.html"),
        "<p>shared base</p>\n",
    )
    .expect("write base file");
    run_projector(&repo_a, &["sync"]);
    run_projector(&repo_b, &["sync"]);

    fs::write(
        repo_a.join("private/briefs/converge.html"),
        "<p>repo a edit</p>\n",
    )
    .expect("write repo a edit");
    fs::write(
        repo_b.join("private/briefs/converge.html"),
        "<p>repo b edit</p>\n",
    )
    .expect("write repo b edit");

    run_projector(&repo_a, &["sync"]);
    run_projector(&repo_b, &["sync"]);
    run_projector(&repo_a, &["sync"]);

    let merged_a =
        fs::read_to_string(repo_a.join("private/briefs/converge.html")).expect("read merged a");
    let merged_b =
        fs::read_to_string(repo_b.join("private/briefs/converge.html")).expect("read merged b");

    assert_eq!(merged_a, merged_b);
    assert!(!merged_a.contains("<<<<<<< existing"));
    assert!(merged_a.contains("repo a edit"));
    assert!(merged_a.contains("repo b edit"));
}
