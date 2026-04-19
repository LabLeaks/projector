use std::path::PathBuf;
use std::process::Command;

use projector_domain::{
    CreateDocumentRequest, RestoreDocumentBodyRevisionRequest, SyncEntryKind, UpdateDocumentRequest,
};
use projector_server::{PostgresWorkspaceStore, WorkspaceStore};
use tokio_postgres::NoTls;

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

async fn wait_for_postgres_store(postgres_url: &str) {
    for _ in 0..120 {
        if PostgresWorkspaceStore::connect(postgres_url).await.is_ok() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    panic!("postgres store did not become reachable");
}

async fn connect_sql_client(
    postgres_url: &str,
) -> (tokio_postgres::Client, tokio::task::JoinHandle<()>) {
    let (client, connection) = tokio_postgres::connect(postgres_url, NoTls)
        .await
        .expect("connect sql client");
    let handle = tokio::spawn(async move {
        if let Err(err) = connection.await {
            panic!("postgres sql connection error: {err}");
        }
    });
    (client, handle)
}

async fn query_i64(
    client: &tokio_postgres::Client,
    sql: &str,
    params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
) -> i64 {
    client
        .query_one(sql, params)
        .await
        .expect("query scalar")
        .get::<_, i64>(0)
}

async fn query_bool(
    client: &tokio_postgres::Client,
    sql: &str,
    params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
) -> bool {
    client
        .query_one(sql, params)
        .await
        .expect("query scalar")
        .get::<_, bool>(0)
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires local docker"]
async fn postgres_internal_checkpoint_metadata_compacts_update_log_and_still_replays() {
    let postgres = DockerPostgres::start();
    wait_for_postgres_store(&postgres.postgres_url).await;
    let store = PostgresWorkspaceStore::connect(&postgres.postgres_url)
        .await
        .expect("connect store");
    let (sql, _connection) = connect_sql_client(&postgres.postgres_url).await;

    let workspace_id = "ws-postgres-internal-compaction";
    let actor_id = "actor-postgres-internal-compaction";
    let mounts = vec![PathBuf::from("private")];
    let (_snapshot, cursor) = store
        .bootstrap_workspace(
            workspace_id,
            &mounts,
            Some("projector"),
            Some(SyncEntryKind::Directory),
        )
        .await
        .expect("bootstrap workspace");

    let document_id = store
        .create_document(&CreateDocumentRequest {
            workspace_id: workspace_id.to_owned(),
            actor_id: actor_id.to_owned(),
            based_on_cursor: Some(cursor),
            mount_relative_path: "private".to_owned(),
            relative_path: "briefs/internal-compaction.html".to_owned(),
            text: "<p>created revision</p>\n".to_owned(),
        })
        .await
        .expect("create document");

    store
        .update_document(&UpdateDocumentRequest {
            workspace_id: workspace_id.to_owned(),
            actor_id: actor_id.to_owned(),
            document_id: document_id.as_str().to_owned(),
            base_text: "<p>created revision</p>\n".to_owned(),
            text: "<p>updated revision one</p>\n".to_owned(),
        })
        .await
        .expect("write first update");
    store
        .update_document(&UpdateDocumentRequest {
            workspace_id: workspace_id.to_owned(),
            actor_id: actor_id.to_owned(),
            document_id: document_id.as_str().to_owned(),
            base_text: "<p>updated revision one</p>\n".to_owned(),
            text: "<p>updated revision two</p>\n".to_owned(),
        })
        .await
        .expect("write second update");

    assert_eq!(
        query_i64(
            &sql,
            "select count(*) from document_body_updates where workspace_id = $1 and document_id = $2",
            &[&workspace_id, &document_id.as_str()],
        )
        .await,
        2
    );
    assert_eq!(
        query_i64(
            &sql,
            "select compacted_through_seq from document_body_snapshots where workspace_id = $1 and document_id = $2",
            &[&workspace_id, &document_id.as_str()],
        )
        .await,
        0
    );
    assert!(
        query_bool(
            &sql,
            "select octet_length(yjs_state) > 0 from document_body_snapshots where workspace_id = $1 and document_id = $2",
            &[&workspace_id, &document_id.as_str()],
        )
        .await
    );

    let (pre_restore_snapshot, _) = store
        .bootstrap_workspace(
            workspace_id,
            &mounts,
            Some("projector"),
            Some(SyncEntryKind::Directory),
        )
        .await
        .expect("bootstrap pre-restore workspace");
    assert_eq!(
        pre_restore_snapshot
            .bodies
            .iter()
            .find(|body| body.document_id == document_id)
            .expect("pre-restore body")
            .text,
        "<p>updated revision two</p>\n"
    );

    let latest_visible_revision = store
        .list_body_revisions(workspace_id, document_id.as_str(), 10)
        .await
        .expect("list body revisions before checkpoint")
        .into_iter()
        .last()
        .expect("latest visible body revision");
    let raw_revision_rows = sql
        .query(
            "select seq, checkpoint_anchor_seq, history_kind \
             from document_body_revisions \
             where workspace_id = $1 and document_id = $2 \
             order by seq asc",
            &[&workspace_id, &document_id.as_str()],
        )
        .await
        .expect("query raw body revision rows");
    assert_eq!(raw_revision_rows.len(), 3);
    assert_eq!(
        raw_revision_rows
            .last()
            .expect("latest raw body revision row")
            .get::<_, i64>("seq") as u64,
        latest_visible_revision.seq
    );

    store
        .restore_document_body_revision(&RestoreDocumentBodyRevisionRequest {
            workspace_id: workspace_id.to_owned(),
            actor_id: actor_id.to_owned(),
            document_id: document_id.as_str().to_owned(),
            seq: latest_visible_revision.seq,
            target_mount_relative_path: None,
            target_relative_path: None,
        })
        .await
        .expect("restore latest body revision as explicit checkpoint");

    assert_eq!(
        query_i64(
            &sql,
            "select count(*) from document_body_updates where workspace_id = $1 and document_id = $2",
            &[&workspace_id, &document_id.as_str()],
        )
        .await,
        0
    );
    assert_eq!(
        query_i64(
            &sql,
            "select compacted_through_seq from document_body_snapshots where workspace_id = $1 and document_id = $2",
            &[&workspace_id, &document_id.as_str()],
        )
        .await,
        2
    );

    let (checkpointed_snapshot, _) = store
        .bootstrap_workspace(
            workspace_id,
            &mounts,
            Some("projector"),
            Some(SyncEntryKind::Directory),
        )
        .await
        .expect("bootstrap checkpointed workspace");
    assert_eq!(
        checkpointed_snapshot
            .bodies
            .iter()
            .find(|body| body.document_id == document_id)
            .expect("checkpointed body")
            .text,
        latest_visible_revision.body_text
    );

    store
        .update_document(&UpdateDocumentRequest {
            workspace_id: workspace_id.to_owned(),
            actor_id: actor_id.to_owned(),
            document_id: document_id.as_str().to_owned(),
            base_text: checkpointed_snapshot
                .bodies
                .iter()
                .find(|body| body.document_id == document_id)
                .expect("checkpointed body")
                .text
                .clone(),
            text: "<p>updated revision three</p>\n".to_owned(),
        })
        .await
        .expect("write third update");

    assert_eq!(
        query_i64(
            &sql,
            "select count(*) from document_body_updates where workspace_id = $1 and document_id = $2",
            &[&workspace_id, &document_id.as_str()],
        )
        .await,
        1
    );

    let revisions = store
        .list_body_revisions(workspace_id, document_id.as_str(), 10)
        .await
        .expect("list body revisions");
    assert_eq!(
        revisions.last().expect("latest body revision").body_text,
        "<p>updated revision three</p>\n"
    );

    let (latest_snapshot, _) = store
        .bootstrap_workspace(
            workspace_id,
            &mounts,
            Some("projector"),
            Some(SyncEntryKind::Directory),
        )
        .await
        .expect("bootstrap latest workspace");
    assert_eq!(
        latest_snapshot
            .bodies
            .iter()
            .find(|body| body.document_id == document_id)
            .expect("latest body")
            .text,
        "<p>updated revision three</p>\n"
    );
}
