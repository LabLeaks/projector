/**
@module PROJECTOR.SERVER.HTTP_RUNTIME
Owns listener adaptation, store-backed server startup, and background file-backed HTTP server spawning for tests and local development.
*/
// @fileimplements PROJECTOR.SERVER.HTTP_RUNTIME
use std::io;
use std::net::TcpListener as StdTcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use crate::{FileWorkspaceStore, PostgresWorkspaceStore, SqliteWorkspaceStore, WorkspaceStore};

use super::handlers;

pub async fn serve(listener: StdTcpListener, store: Arc<dyn WorkspaceStore>) -> io::Result<()> {
    listener.set_nonblocking(true)?;
    let listener = tokio::net::TcpListener::from_std(listener)?;
    axum::serve(listener, handlers::app(store)).await
}

pub async fn serve_file_backed(listener: StdTcpListener, state_dir: PathBuf) -> io::Result<()> {
    serve(listener, Arc::new(FileWorkspaceStore::new(state_dir))).await
}

pub async fn serve_sqlite(listener: StdTcpListener, sqlite_path: PathBuf) -> io::Result<()> {
    let store = SqliteWorkspaceStore::connect(sqlite_path).map_err(io::Error::other)?;
    serve(listener, Arc::new(store)).await
}

pub async fn serve_postgres(listener: StdTcpListener, postgres_url: String) -> io::Result<()> {
    let store = PostgresWorkspaceStore::connect(&postgres_url)
        .await
        .map_err(io::Error::other)?;
    serve(listener, Arc::new(store)).await
}

pub fn spawn_background(listener: StdTcpListener, state_dir: PathBuf) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
        runtime
            .block_on(serve_file_backed(listener, state_dir))
            .expect("serve projector server");
    })
}
