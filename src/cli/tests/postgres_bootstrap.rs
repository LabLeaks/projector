use std::fs;
use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use projector_domain::{
    ActorId, CheckoutBinding, DocumentBodyRevision, DocumentPathRevision, ListBodyRevisionsRequest,
    ListBodyRevisionsResponse, ListPathRevisionsRequest, ListPathRevisionsResponse,
    ProjectionRoots, ReconstructWorkspaceRequest, ReconstructWorkspaceResponse,
    ResolveHistoricalPathRequest, ResolveHistoricalPathResponse, RestoreWorkspaceRequest,
};
use projector_runtime::{BindingStore, FileBindingStore, HttpTransport, Transport};

fn temp_repo(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time before unix epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("projector-{name}-{unique}"));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create temp repo root");
    let status = Command::new("git")
        .arg("init")
        .arg("-q")
        .current_dir(&root)
        .status()
        .expect("git init");
    assert!(status.success(), "git init failed");
    fs::create_dir_all(root.join(".jj")).expect("create fake jj repo");
    root
}

fn run_projector(repo_root: &Path, args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_projector"))
        .args(args)
        .current_dir(repo_root)
        .output()
        .expect("run projector");
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf8 stdout")
}

fn list_body_revisions(
    addr: &str,
    workspace_id: &str,
    document_id: &str,
    limit: usize,
) -> Vec<DocumentBodyRevision> {
    reqwest::blocking::Client::new()
        .post(format!("http://{addr}/history/body/list"))
        .json(&ListBodyRevisionsRequest {
            workspace_id: workspace_id.to_owned(),
            document_id: document_id.to_owned(),
            limit,
        })
        .send()
        .expect("send body history request")
        .error_for_status()
        .expect("body history response status")
        .json::<ListBodyRevisionsResponse>()
        .expect("decode body history response")
        .revisions
}

fn list_path_revisions(
    addr: &str,
    workspace_id: &str,
    document_id: &str,
    limit: usize,
) -> Vec<DocumentPathRevision> {
    reqwest::blocking::Client::new()
        .post(format!("http://{addr}/history/path/list"))
        .json(&ListPathRevisionsRequest {
            workspace_id: workspace_id.to_owned(),
            document_id: document_id.to_owned(),
            limit,
        })
        .send()
        .expect("send path history request")
        .error_for_status()
        .expect("path history response status")
        .json::<ListPathRevisionsResponse>()
        .expect("decode path history response")
        .revisions
}

fn resolve_document_by_historical_path(
    addr: &str,
    workspace_id: &str,
    mount_relative_path: &str,
    relative_path: &str,
) -> String {
    reqwest::blocking::Client::new()
        .post(format!("http://{addr}/history/path/resolve"))
        .json(&ResolveHistoricalPathRequest {
            workspace_id: workspace_id.to_owned(),
            mount_relative_path: mount_relative_path.to_owned(),
            relative_path: relative_path.to_owned(),
        })
        .send()
        .expect("send historical path resolve request")
        .error_for_status()
        .expect("historical path resolve response status")
        .json::<ResolveHistoricalPathResponse>()
        .expect("decode historical path resolve response")
        .document_id
}

fn reconstruct_workspace_at_cursor(
    addr: &str,
    workspace_id: &str,
    cursor: u64,
) -> projector_domain::BootstrapSnapshot {
    reqwest::blocking::Client::new()
        .post(format!("http://{addr}/history/workspace/reconstruct"))
        .json(&ReconstructWorkspaceRequest {
            workspace_id: workspace_id.to_owned(),
            cursor,
        })
        .send()
        .expect("send workspace reconstruction request")
        .error_for_status()
        .expect("workspace reconstruction response status")
        .json::<ReconstructWorkspaceResponse>()
        .expect("decode workspace reconstruction response")
        .snapshot
}

fn restore_workspace_at_cursor(
    addr: &str,
    workspace_id: &str,
    actor_id: &str,
    based_on_cursor: u64,
    cursor: u64,
) {
    reqwest::blocking::Client::new()
        .post(format!("http://{addr}/history/workspace/restore"))
        .json(&RestoreWorkspaceRequest {
            workspace_id: workspace_id.to_owned(),
            actor_id: actor_id.to_owned(),
            based_on_cursor: Some(based_on_cursor),
            cursor,
        })
        .send()
        .expect("send workspace restore request")
        .error_for_status()
        .expect("workspace restore response status");
}

fn clone_binding_for_repo(
    binding: &CheckoutBinding,
    repo_root: &Path,
    actor_id: &str,
) -> CheckoutBinding {
    CheckoutBinding {
        workspace_id: binding.workspace_id.clone(),
        actor_id: ActorId::new(actor_id),
        server_addr: binding.server_addr.clone(),
        roots: ProjectionRoots {
            projector_dir: repo_root.join(".projector"),
            projection_paths: binding
                .projection_relative_paths
                .iter()
                .map(|path| repo_root.join(path))
                .collect(),
        },
        projection_relative_paths: binding.projection_relative_paths.clone(),
        projection_kinds: binding.projection_kinds.clone(),
    }
}

fn spawn_postgres_server(postgres_url: &str) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind server addr");
    let addr = listener.local_addr().expect("local addr");
    let postgres_url = postgres_url.to_owned();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
        runtime
            .block_on(projector_server::serve_postgres(listener, postgres_url))
            .expect("serve projector server");
    });
    std::thread::sleep(std::time::Duration::from_millis(250));
    addr
}

fn wait_for_postgres_store(postgres_url: &str) {
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    for _ in 0..120 {
        if runtime
            .block_on(projector_server::PostgresWorkspaceStore::connect(
                postgres_url,
            ))
            .is_ok()
        {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    panic!("postgres store did not become reachable");
}

struct DockerPostgres {
    container_id: String,
    postgres_url: String,
}

impl DockerPostgres {
    fn start() -> Self {
        let output = Command::new("docker")
            .args([
                "run",
                "-d",
                "--rm",
                "-e",
                "POSTGRES_USER=projector",
                "-e",
                "POSTGRES_PASSWORD=projector",
                "-e",
                "POSTGRES_DB=projector",
                "-P",
                "postgres:17-alpine",
            ])
            .output()
            .expect("start postgres container");
        assert!(
            output.status.success(),
            "docker run failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let container_id = String::from_utf8(output.stdout)
            .expect("utf8 container id")
            .trim()
            .to_owned();

        let host_port = wait_for_postgres_port(&container_id);
        wait_for_postgres_ready(&container_id);

        Self {
            container_id,
            postgres_url: format!("postgres://projector:projector@127.0.0.1:{host_port}/projector"),
        }
    }

    fn query_scalar(&self, sql: &str) -> String {
        let output = Command::new("docker")
            .args([
                "exec",
                "-e",
                "PGPASSWORD=projector",
                &self.container_id,
                "psql",
                "-v",
                "ON_ERROR_STOP=1",
                "-At",
                "-U",
                "projector",
                "-d",
                "projector",
                "-c",
                sql,
            ])
            .output()
            .expect("query postgres scalar");
        assert!(
            output.status.success(),
            "psql query failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .expect("utf8 query output")
            .trim()
            .to_owned()
    }
}

impl Drop for DockerPostgres {
    fn drop(&mut self) {
        let _ = Command::new("docker")
            .args(["rm", "-f", &self.container_id])
            .status();
    }
}

fn wait_for_postgres_port(container_id: &str) -> String {
    for _ in 0..120 {
        let output = Command::new("docker")
            .args(["port", container_id, "5432/tcp"])
            .output()
            .expect("inspect postgres port");
        if output.status.success() {
            let stdout = String::from_utf8(output.stdout).expect("utf8 docker port");
            if let Some(line) = stdout.lines().find(|line| !line.trim().is_empty()) {
                if let Some(port) = line.rsplit(':').next() {
                    return port.trim().to_owned();
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    panic!("postgres container did not publish a host port");
}

fn wait_for_postgres_ready(container_id: &str) {
    for _ in 0..120 {
        let output = Command::new("docker")
            .args([
                "exec",
                container_id,
                "pg_isready",
                "-U",
                "projector",
                "-d",
                "projector",
            ])
            .output()
            .expect("check postgres readiness");
        if output.status.success() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    panic!("postgres container did not become ready");
}

// @verifies PROJECTOR.SERVER.DOCUMENTS.CREATE_TRANSACTIONAL_DOCUMENT
#[test]
#[ignore = "requires local docker"]
fn sync_applies_initial_snapshot_from_postgres_server() {
    let postgres = DockerPostgres::start();
    wait_for_postgres_store(&postgres.postgres_url);
    let addr = spawn_postgres_server(&postgres.postgres_url).to_string();

    let repo = temp_repo("postgres-bootstrap");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");

    let first_sync = run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    let workspace_id = first_sync
        .lines()
        .find_map(|line| line.strip_prefix("workspace_id: "))
        .expect("workspace id")
        .to_owned();
    let binding = FileBindingStore::new(&repo)
        .load()
        .expect("load binding")
        .expect("bound checkout");
    let mut transport = HttpTransport::new(format!("http://{addr}"));
    let (_snapshot, cursor) = transport.bootstrap(&binding).expect("bootstrap");

    let document_id = transport
        .create_document(
            &binding,
            cursor,
            Path::new("private"),
            Path::new("briefs/index.html"),
            "<h1>Today</h1>\n<p>From Postgres.</p>\n",
        )
        .expect("create document through server");

    let second_sync = run_projector(&repo, &["sync"]);

    assert!(second_sync.contains("binding: reused"));
    assert_eq!(
        fs::read_to_string(repo.join("private/briefs/index.html")).expect("read materialized file"),
        "<h1>Today</h1>\n<p>From Postgres.</p>\n"
    );
    assert_eq!(
        postgres.query_scalar(&format!(
            "select count(*) from provenance_events where workspace_id = '{workspace_id}' and document_id = '{}' and event_kind = 'document_created'",
            document_id.as_str()
        )),
        "1"
    );
}

// @verifies PROJECTOR.SERVER.SYNC.CHANGES_SINCE_RETURNS_CHANGED_DOCUMENTS
#[test]
#[ignore = "requires local docker"]
fn changes_since_returns_only_changed_documents_from_postgres_server() {
    let postgres = DockerPostgres::start();
    wait_for_postgres_store(&postgres.postgres_url);
    let addr = spawn_postgres_server(&postgres.postgres_url).to_string();

    let repo = temp_repo("postgres-changes-since");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");

    run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    let binding = FileBindingStore::new(&repo)
        .load()
        .expect("load binding")
        .expect("bound checkout");
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
        .expect("create document through server");

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

// @verifies PROJECTOR.SERVER.DOCUMENTS.UPDATE_TRANSACTIONAL_DOCUMENT
#[test]
#[ignore = "requires local docker"]
fn sync_updates_text_document_through_postgres_server() {
    let postgres = DockerPostgres::start();
    wait_for_postgres_store(&postgres.postgres_url);
    let addr = spawn_postgres_server(&postgres.postgres_url).to_string();

    let repo = temp_repo("postgres-update");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");

    let first_sync = run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    let workspace_id = first_sync
        .lines()
        .find_map(|line| line.strip_prefix("workspace_id: "))
        .expect("workspace id")
        .to_owned();
    let binding = FileBindingStore::new(&repo)
        .load()
        .expect("load binding")
        .expect("bound checkout");
    let mut transport = HttpTransport::new(format!("http://{addr}"));
    let (_snapshot, cursor) = transport.bootstrap(&binding).expect("bootstrap");

    let document_id = transport
        .create_document(
            &binding,
            cursor,
            Path::new("private"),
            Path::new("briefs/index.html"),
            "<h1>Today</h1>\n<p>From Postgres.</p>\n",
        )
        .expect("create document through server");

    run_projector(&repo, &["sync"]);
    fs::write(
        repo.join("private/briefs/index.html"),
        "<h1>Today</h1>\n<p>Updated from local sync.</p>\n",
    )
    .expect("write updated local file");
    run_projector(&repo, &["sync"]);

    assert_eq!(
        postgres.query_scalar(&format!(
            "select body_text from document_body_snapshots where workspace_id = '{workspace_id}' and document_id = '{}'",
            document_id.as_str()
        )),
        "<h1>Today</h1>\n<p>Updated from local sync.</p>"
    );
    assert_eq!(
        postgres.query_scalar(&format!(
            "select count(*) from provenance_events where workspace_id = '{workspace_id}' and document_id = '{}' and event_kind = 'document_updated'",
            document_id.as_str()
        )),
        "1"
    );
}

// @verifies PROJECTOR.SERVER.DOCUMENTS.DELETE_TRANSACTIONAL_DOCUMENT
#[test]
#[ignore = "requires local docker"]
fn sync_deletes_text_document_through_postgres_server() {
    let postgres = DockerPostgres::start();
    wait_for_postgres_store(&postgres.postgres_url);
    let addr = spawn_postgres_server(&postgres.postgres_url).to_string();

    let repo = temp_repo("postgres-delete");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");

    let first_sync = run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    let workspace_id = first_sync
        .lines()
        .find_map(|line| line.strip_prefix("workspace_id: "))
        .expect("workspace id")
        .to_owned();
    let binding = FileBindingStore::new(&repo)
        .load()
        .expect("load binding")
        .expect("bound checkout");
    let mut transport = HttpTransport::new(format!("http://{addr}"));
    let (_snapshot, cursor) = transport.bootstrap(&binding).expect("bootstrap");

    let document_id = transport
        .create_document(
            &binding,
            cursor,
            Path::new("private"),
            Path::new("briefs/index.html"),
            "<h1>Delete Me</h1>\n",
        )
        .expect("create document through server");

    run_projector(&repo, &["sync"]);
    fs::remove_file(repo.join("private/briefs/index.html")).expect("remove local file");
    run_projector(&repo, &["sync"]);

    assert_eq!(
        postgres.query_scalar(&format!(
            "select deleted::text from document_paths where workspace_id = '{workspace_id}' and document_id = '{}'",
            document_id.as_str()
        )),
        "true"
    );
    assert_eq!(
        postgres.query_scalar(&format!(
            "select count(*) from provenance_events where workspace_id = '{workspace_id}' and document_id = '{}' and event_kind = 'document_deleted'",
            document_id.as_str()
        )),
        "1"
    );
}

// @verifies PROJECTOR.SERVER.DOCUMENTS.MOVE_TRANSACTIONAL_DOCUMENT
#[test]
#[ignore = "requires local docker"]
fn sync_moves_text_document_through_postgres_server() {
    let postgres = DockerPostgres::start();
    wait_for_postgres_store(&postgres.postgres_url);
    let addr = spawn_postgres_server(&postgres.postgres_url).to_string();

    let repo = temp_repo("postgres-move");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");

    let first_sync = run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    let workspace_id = first_sync
        .lines()
        .find_map(|line| line.strip_prefix("workspace_id: "))
        .expect("workspace id")
        .to_owned();
    let binding = FileBindingStore::new(&repo)
        .load()
        .expect("load binding")
        .expect("bound checkout");
    let mut transport = HttpTransport::new(format!("http://{addr}"));
    let (_snapshot, cursor) = transport.bootstrap(&binding).expect("bootstrap");

    let document_id = transport
        .create_document(
            &binding,
            cursor,
            Path::new("private"),
            Path::new("briefs/index.html"),
            "<h1>Move Me</h1>\n",
        )
        .expect("create document through server");

    run_projector(&repo, &["sync"]);
    fs::create_dir_all(repo.join("private/archive")).expect("create archive dir");
    fs::rename(
        repo.join("private/briefs/index.html"),
        repo.join("private/archive/index.html"),
    )
    .expect("rename local file");
    run_projector(&repo, &["sync"]);

    assert_eq!(
        postgres.query_scalar(&format!(
            "select mount_path || '/' || relative_path from document_paths where workspace_id = '{workspace_id}' and document_id = '{}'",
            document_id.as_str()
        )),
        "private/archive/index.html"
    );
    assert_eq!(
        postgres.query_scalar(&format!(
            "select count(*) from provenance_events where workspace_id = '{workspace_id}' and document_id = '{}' and event_kind = 'document_moved'",
            document_id.as_str()
        )),
        "1"
    );
}

// @verifies PROJECTOR.HISTORY.DOCUMENT_BODY_HISTORY
#[test]
#[ignore = "requires local docker"]
fn postgres_server_retains_document_body_revisions() {
    let postgres = DockerPostgres::start();
    wait_for_postgres_store(&postgres.postgres_url);
    let addr = spawn_postgres_server(&postgres.postgres_url).to_string();

    let repo = temp_repo("postgres-body-history");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");

    let first_sync = run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    let workspace_id = first_sync
        .lines()
        .find_map(|line| line.strip_prefix("workspace_id: "))
        .expect("workspace id")
        .to_owned();

    fs::create_dir_all(repo.join("private/briefs")).expect("create local subdir");
    fs::write(
        repo.join("private/briefs/history.html"),
        "<p>created revision</p>\n",
    )
    .expect("write create");
    run_projector(&repo, &["sync"]);

    fs::write(
        repo.join("private/briefs/history.html"),
        "<p>updated revision</p>\n",
    )
    .expect("write update");
    run_projector(&repo, &["sync"]);

    assert_eq!(
        postgres.query_scalar(&format!(
            "select count(*) from document_body_revisions where workspace_id = '{workspace_id}'"
        )),
        "2"
    );
    assert_eq!(
        postgres.query_scalar(&format!(
            "select base_text from document_body_revisions where workspace_id = '{workspace_id}' order by seq asc limit 1"
        )),
        ""
    );
    assert_eq!(
        postgres.query_scalar(&format!(
            "select body_text from document_body_revisions where workspace_id = '{workspace_id}' order by seq desc limit 1"
        )),
        "<p>updated revision</p>"
    );
}

// @verifies PROJECTOR.SERVER.HISTORY.LISTS_DOCUMENT_BODY_REVISIONS
#[test]
#[ignore = "requires local docker"]
fn postgres_server_lists_document_body_revisions() {
    let postgres = DockerPostgres::start();
    wait_for_postgres_store(&postgres.postgres_url);
    let addr = spawn_postgres_server(&postgres.postgres_url).to_string();

    let repo = temp_repo("postgres-body-history-list");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");

    let first_sync = run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    let workspace_id = first_sync
        .lines()
        .find_map(|line| line.strip_prefix("workspace_id: "))
        .expect("workspace id")
        .to_owned();

    fs::create_dir_all(repo.join("private/briefs")).expect("create local subdir");
    fs::write(
        repo.join("private/briefs/history-list.html"),
        "<p>created revision</p>\n",
    )
    .expect("write create");
    run_projector(&repo, &["sync"]);

    let binding = FileBindingStore::new(&repo)
        .load()
        .expect("load binding")
        .expect("bound checkout");
    let mut transport = HttpTransport::new(format!("http://{addr}"));
    let (snapshot, _) = transport.bootstrap(&binding).expect("bootstrap");
    let document_id = snapshot
        .manifest
        .entries
        .iter()
        .find(|entry| {
            !entry.deleted
                && entry.mount_relative_path == Path::new("private")
                && entry.relative_path == Path::new("briefs/history-list.html")
        })
        .expect("created entry")
        .document_id
        .as_str()
        .to_owned();

    fs::write(
        repo.join("private/briefs/history-list.html"),
        "<p>updated revision</p>\n",
    )
    .expect("write update");
    run_projector(&repo, &["sync"]);

    let revisions = list_body_revisions(&addr, &workspace_id, &document_id, 10);
    assert_eq!(revisions.len(), 2);
    assert_eq!(revisions[0].base_text, "");
    assert_eq!(revisions[0].body_text, "<p>created revision</p>\n");
    assert_eq!(revisions[1].base_text, "<p>created revision</p>\n");
    assert_eq!(revisions[1].body_text, "<p>updated revision</p>\n");
}

// @verifies PROJECTOR.HISTORY.MANIFEST_PATH_HISTORY
#[test]
#[ignore = "requires local docker"]
fn postgres_server_retains_document_path_history() {
    let postgres = DockerPostgres::start();
    wait_for_postgres_store(&postgres.postgres_url);
    let addr = spawn_postgres_server(&postgres.postgres_url).to_string();

    let repo = temp_repo("postgres-path-history");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");

    let first_sync = run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    let workspace_id = first_sync
        .lines()
        .find_map(|line| line.strip_prefix("workspace_id: "))
        .expect("workspace id")
        .to_owned();

    fs::create_dir_all(repo.join("private/briefs")).expect("create local subdir");
    fs::write(
        repo.join("private/briefs/path-history.html"),
        "<p>create then move then delete</p>\n",
    )
    .expect("write create");
    run_projector(&repo, &["sync"]);

    fs::create_dir_all(repo.join("notes/archive")).expect("create move target");
    fs::rename(
        repo.join("private/briefs/path-history.html"),
        repo.join("notes/archive/path-history.html"),
    )
    .expect("move file");
    run_projector(&repo, &["sync"]);

    fs::remove_file(repo.join("notes/archive/path-history.html")).expect("delete moved file");
    run_projector(&repo, &["sync"]);

    assert_eq!(
        postgres.query_scalar(&format!(
            "select count(*) from document_path_history where workspace_id = '{workspace_id}'"
        )),
        "3"
    );
    assert_eq!(
        postgres.query_scalar(&format!(
            "select string_agg(event_kind, ',' order by seq) from document_path_history where workspace_id = '{workspace_id}'"
        )),
        "document_created,document_moved,document_deleted"
    );
}

// @verifies PROJECTOR.SERVER.HISTORY.LISTS_DOCUMENT_PATH_REVISIONS
#[test]
#[ignore = "requires local docker"]
fn postgres_server_lists_document_path_revisions() {
    let postgres = DockerPostgres::start();
    wait_for_postgres_store(&postgres.postgres_url);
    let addr = spawn_postgres_server(&postgres.postgres_url).to_string();

    let repo = temp_repo("postgres-path-history-list");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");

    let first_sync = run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    let workspace_id = first_sync
        .lines()
        .find_map(|line| line.strip_prefix("workspace_id: "))
        .expect("workspace id")
        .to_owned();

    fs::create_dir_all(repo.join("private/briefs")).expect("create local subdir");
    fs::write(
        repo.join("private/briefs/path-history-list.html"),
        "<p>create then move then delete</p>\n",
    )
    .expect("write create");
    run_projector(&repo, &["sync"]);

    let binding = FileBindingStore::new(&repo)
        .load()
        .expect("load binding")
        .expect("bound checkout");
    let mut transport = HttpTransport::new(format!("http://{addr}"));
    let (snapshot, _) = transport.bootstrap(&binding).expect("bootstrap");
    let document_id = snapshot
        .manifest
        .entries
        .iter()
        .find(|entry| {
            !entry.deleted
                && entry.mount_relative_path == Path::new("private")
                && entry.relative_path == Path::new("briefs/path-history-list.html")
        })
        .expect("created entry")
        .document_id
        .as_str()
        .to_owned();

    fs::create_dir_all(repo.join("notes/archive")).expect("create move target");
    fs::rename(
        repo.join("private/briefs/path-history-list.html"),
        repo.join("notes/archive/path-history-list.html"),
    )
    .expect("move file");
    run_projector(&repo, &["sync"]);

    fs::remove_file(repo.join("notes/archive/path-history-list.html")).expect("delete moved file");
    run_projector(&repo, &["sync"]);

    let revisions = list_path_revisions(&addr, &workspace_id, &document_id, 10);
    assert_eq!(revisions.len(), 3);
    assert_eq!(revisions[0].event_kind, "document_created");
    assert_eq!(revisions[0].mount_path, "private");
    assert_eq!(revisions[0].relative_path, "briefs/path-history-list.html");
    assert_eq!(revisions[1].event_kind, "document_moved");
    assert_eq!(revisions[1].mount_path, "notes");
    assert_eq!(revisions[1].relative_path, "archive/path-history-list.html");
    assert_eq!(revisions[2].event_kind, "document_deleted");
    assert!(revisions[2].deleted);
}

// @verifies PROJECTOR.SERVER.HISTORY.RESTORES_DOCUMENT_BODY_REVISION
#[test]
#[ignore = "requires local docker"]
fn postgres_server_restores_document_body_revision() {
    let postgres = DockerPostgres::start();
    wait_for_postgres_store(&postgres.postgres_url);
    let addr = spawn_postgres_server(&postgres.postgres_url).to_string();

    let repo = temp_repo("postgres-body-restore");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");

    let first_sync = run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    let workspace_id = first_sync
        .lines()
        .find_map(|line| line.strip_prefix("workspace_id: "))
        .expect("workspace id")
        .to_owned();
    let binding = FileBindingStore::new(&repo)
        .load()
        .expect("load binding")
        .expect("bound checkout");
    let mut transport = HttpTransport::new(format!("http://{addr}"));

    fs::create_dir_all(repo.join("private/briefs")).expect("create local subdir");
    fs::write(
        repo.join("private/briefs/restore-postgres.html"),
        "<p>created revision</p>\n",
    )
    .expect("write create");
    run_projector(&repo, &["sync"]);

    let (snapshot, _) = transport.bootstrap(&binding).expect("bootstrap");
    let entry = snapshot
        .manifest
        .entries
        .iter()
        .find(|entry| {
            !entry.deleted
                && entry.mount_relative_path == Path::new("private")
                && entry.relative_path == Path::new("briefs/restore-postgres.html")
        })
        .expect("created entry");

    fs::write(
        repo.join("private/briefs/restore-postgres.html"),
        "<p>updated revision</p>\n",
    )
    .expect("write update");
    run_projector(&repo, &["sync"]);

    transport
        .restore_document_body_revision(&binding, &entry.document_id, 1, None, None)
        .expect("restore body revision");

    let (restored_snapshot, _) = transport.bootstrap(&binding).expect("bootstrap restored");
    let restored_body = restored_snapshot
        .bodies
        .iter()
        .find(|body| body.document_id == entry.document_id)
        .expect("restored body");
    assert_eq!(restored_body.text, "<p>created revision</p>\n");

    let revisions = list_body_revisions(&addr, &workspace_id, entry.document_id.as_str(), 10);
    assert_eq!(revisions.len(), 3);
    assert_eq!(revisions[2].base_text, "<p>updated revision</p>\n");
    assert_eq!(revisions[2].body_text, "<p>created revision</p>\n");
    assert!(!revisions[2].conflicted);
}

// @verifies PROJECTOR.SERVER.HISTORY.REVIVES_DELETED_DOCUMENT_AT_LAST_PATH
#[test]
#[ignore = "requires local docker"]
fn postgres_server_restore_revives_deleted_document_at_last_path() {
    let postgres = DockerPostgres::start();
    wait_for_postgres_store(&postgres.postgres_url);
    let addr = spawn_postgres_server(&postgres.postgres_url).to_string();

    let repo = temp_repo("postgres-restore-deleted");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");

    run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    let binding = FileBindingStore::new(&repo)
        .load()
        .expect("load binding")
        .expect("bound checkout");
    let mut transport = HttpTransport::new(format!("http://{addr}"));

    fs::create_dir_all(repo.join("private/briefs")).expect("create local subdir");
    fs::write(
        repo.join("private/briefs/restore-deleted-postgres.html"),
        "<p>created revision</p>\n",
    )
    .expect("write create");
    run_projector(&repo, &["sync"]);

    let (snapshot, _) = transport.bootstrap(&binding).expect("bootstrap");
    let entry = snapshot
        .manifest
        .entries
        .iter()
        .find(|entry| {
            !entry.deleted
                && entry.mount_relative_path == Path::new("private")
                && entry.relative_path == Path::new("briefs/restore-deleted-postgres.html")
        })
        .expect("created entry");

    fs::write(
        repo.join("private/briefs/restore-deleted-postgres.html"),
        "<p>updated revision</p>\n",
    )
    .expect("write update");
    run_projector(&repo, &["sync"]);
    fs::remove_file(repo.join("private/briefs/restore-deleted-postgres.html"))
        .expect("delete local file");
    run_projector(&repo, &["sync"]);

    transport
        .restore_document_body_revision(&binding, &entry.document_id, 1, None, None)
        .expect("restore deleted body revision");

    let (restored_snapshot, _) = transport.bootstrap(&binding).expect("bootstrap restored");
    let restored_entry = restored_snapshot
        .manifest
        .entries
        .iter()
        .find(|candidate| candidate.document_id == entry.document_id)
        .expect("restored entry");
    assert!(!restored_entry.deleted);
    let restored_body = restored_snapshot
        .bodies
        .iter()
        .find(|body| body.document_id == entry.document_id)
        .expect("restored body");
    assert_eq!(restored_body.text, "<p>created revision</p>\n");

    let path_revisions = list_path_revisions(
        &addr,
        binding.workspace_id.as_str(),
        entry.document_id.as_str(),
        10,
    );
    assert_eq!(
        path_revisions
            .last()
            .expect("latest path revision")
            .event_kind,
        "document_restored"
    );
}

// @verifies PROJECTOR.SERVER.HISTORY.RESOLVES_DOCUMENT_BY_HISTORICAL_PATH
#[test]
#[ignore = "requires local docker"]
fn postgres_server_resolves_document_by_historical_moved_path() {
    let postgres = DockerPostgres::start();
    wait_for_postgres_store(&postgres.postgres_url);
    let addr = spawn_postgres_server(&postgres.postgres_url).to_string();

    let repo = temp_repo("postgres-resolve-historical");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");

    let first_sync = run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    let workspace_id = first_sync
        .lines()
        .find_map(|line| line.strip_prefix("workspace_id: "))
        .expect("workspace id")
        .to_owned();

    fs::create_dir_all(repo.join("private/briefs")).expect("create local subdir");
    fs::write(
        repo.join("private/briefs/resolve-historical-postgres.html"),
        "<p>created revision</p>\n",
    )
    .expect("write create");
    run_projector(&repo, &["sync"]);

    let binding = FileBindingStore::new(&repo)
        .load()
        .expect("load binding")
        .expect("bound checkout");
    let mut transport = HttpTransport::new(format!("http://{addr}"));
    let (snapshot, _) = transport.bootstrap(&binding).expect("bootstrap");
    let entry = snapshot
        .manifest
        .entries
        .iter()
        .find(|entry| {
            !entry.deleted
                && entry.mount_relative_path == Path::new("private")
                && entry.relative_path == Path::new("briefs/resolve-historical-postgres.html")
        })
        .expect("created entry");

    fs::create_dir_all(repo.join("notes/archive")).expect("create move target");
    fs::rename(
        repo.join("private/briefs/resolve-historical-postgres.html"),
        repo.join("notes/archive/resolve-historical-postgres.html"),
    )
    .expect("move file");
    run_projector(&repo, &["sync"]);

    let resolved = resolve_document_by_historical_path(
        &addr,
        &workspace_id,
        "private",
        "briefs/resolve-historical-postgres.html",
    );
    assert_eq!(resolved, entry.document_id.as_str());
}

// @verifies PROJECTOR.SERVER.HISTORY.RECONSTRUCTS_WORKSPACE_AT_CURSOR
#[test]
#[ignore = "requires local docker"]
fn postgres_server_reconstructs_workspace_at_cursor() {
    let postgres = DockerPostgres::start();
    wait_for_postgres_store(&postgres.postgres_url);
    let addr = spawn_postgres_server(&postgres.postgres_url).to_string();

    let repo = temp_repo("postgres-workspace-reconstruct");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");

    let first_sync = run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    let workspace_id = first_sync
        .lines()
        .find_map(|line| line.strip_prefix("workspace_id: "))
        .expect("workspace id")
        .to_owned();

    fs::create_dir_all(repo.join("private/briefs")).expect("create local subdir");
    fs::write(
        repo.join("private/briefs/workspace-history.html"),
        "<p>created revision</p>\n",
    )
    .expect("write create");
    run_projector(&repo, &["sync"]);

    fs::write(
        repo.join("private/briefs/workspace-history.html"),
        "<p>updated revision</p>\n",
    )
    .expect("write update");
    run_projector(&repo, &["sync"]);

    fs::create_dir_all(repo.join("notes/archive")).expect("create move target");
    fs::rename(
        repo.join("private/briefs/workspace-history.html"),
        repo.join("notes/archive/workspace-history.html"),
    )
    .expect("move file");
    run_projector(&repo, &["sync"]);

    let reconstructed = reconstruct_workspace_at_cursor(&addr, &workspace_id, 2);

    assert_eq!(reconstructed.manifest.entries.len(), 1);
    let entry = &reconstructed.manifest.entries[0];
    assert!(!entry.deleted);
    assert_eq!(entry.mount_relative_path, Path::new("private"));
    assert_eq!(
        entry.relative_path,
        Path::new("briefs/workspace-history.html")
    );
    assert_eq!(reconstructed.bodies.len(), 1);
    assert_eq!(reconstructed.bodies[0].text, "<p>updated revision</p>\n");
}

// @verifies PROJECTOR.SERVER.HISTORY.RESTORES_WORKSPACE_AT_CURSOR
#[test]
#[ignore = "requires local docker"]
fn postgres_server_restores_workspace_at_cursor() {
    let postgres = DockerPostgres::start();
    wait_for_postgres_store(&postgres.postgres_url);
    let addr = spawn_postgres_server(&postgres.postgres_url).to_string();

    let repo = temp_repo("postgres-workspace-restore");
    fs::write(repo.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");

    let first_sync = run_projector(&repo, &["sync", "--server", &addr, "private", "notes"]);
    let workspace_id = first_sync
        .lines()
        .find_map(|line| line.strip_prefix("workspace_id: "))
        .expect("workspace id")
        .to_owned();
    let actor_id = first_sync
        .lines()
        .find_map(|line| line.strip_prefix("actor_id: "))
        .expect("actor id")
        .to_owned();

    fs::create_dir_all(repo.join("private/briefs")).expect("create local subdir");
    fs::write(
        repo.join("private/briefs/workspace-restore.html"),
        "<p>created revision</p>\n",
    )
    .expect("write create");
    run_projector(&repo, &["sync"]);

    fs::write(
        repo.join("private/briefs/workspace-restore.html"),
        "<p>updated revision</p>\n",
    )
    .expect("write update");
    run_projector(&repo, &["sync"]);

    fs::create_dir_all(repo.join("notes/archive")).expect("create move target");
    fs::rename(
        repo.join("private/briefs/workspace-restore.html"),
        repo.join("notes/archive/workspace-restore.html"),
    )
    .expect("move file");
    run_projector(&repo, &["sync"]);

    restore_workspace_at_cursor(&addr, &workspace_id, &actor_id, 3, 1);

    let binding = FileBindingStore::new(&repo)
        .load()
        .expect("load binding")
        .expect("bound checkout");
    let mut transport = HttpTransport::new(format!("http://{addr}"));
    let (restored_snapshot, _) = transport.bootstrap(&binding).expect("bootstrap restored");
    let entry = restored_snapshot
        .manifest
        .entries
        .iter()
        .find(|entry| !entry.deleted)
        .expect("restored live entry");
    assert_eq!(entry.mount_relative_path, Path::new("private"));
    assert_eq!(
        entry.relative_path,
        Path::new("briefs/workspace-restore.html")
    );
    assert_eq!(restored_snapshot.bodies.len(), 1);
    assert_eq!(
        restored_snapshot.bodies[0].text,
        "<p>created revision</p>\n"
    );
}

// @verifies PROJECTOR.SYNC.TEXT_CONVERGENCE
#[test]
#[ignore = "requires local docker"]
fn sync_converges_concurrent_text_updates_through_postgres_server() {
    let postgres = DockerPostgres::start();
    wait_for_postgres_store(&postgres.postgres_url);
    let addr = spawn_postgres_server(&postgres.postgres_url).to_string();

    let repo_a = temp_repo("postgres-converge-a");
    let repo_b = temp_repo("postgres-converge-b");
    fs::write(repo_a.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");
    fs::write(repo_b.join(".gitignore"), "private/\nnotes/\n").expect("write gitignore");

    run_projector(&repo_a, &["sync", "--server", &addr, "private", "notes"]);
    let binding_a = FileBindingStore::new(&repo_a)
        .load()
        .expect("load repo a binding")
        .expect("bound checkout");
    FileBindingStore::new(&repo_b)
        .save(&clone_binding_for_repo(
            &binding_a,
            &repo_b,
            "actor-postgres-converge-b",
        ))
        .expect("save repo b binding");

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
    assert!(merged_a.contains("<<<<<<< existing"));
    assert!(merged_a.contains("<p>repo a edit</p>"));
    assert!(merged_a.contains("<p>repo b edit</p>"));
    assert!(merged_a.contains(">>>>>>> incoming"));
}
