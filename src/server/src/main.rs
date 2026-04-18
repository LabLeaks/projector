/**
@module PROJECTOR.SERVER.BIN
Starts the projector server with SQLite, Postgres, or file-backed storage from the command-line entrypoint.
*/
// @fileimplements PROJECTOR.SERVER.BIN
use std::env;
use std::error::Error;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process;

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        eprintln!("{err}");
        process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("serve") => run_serve(args.collect()).await,
        _ => {
            print_usage();
            Ok(())
        }
    }
}

async fn run_serve(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    let (addr, store) = parse_serve_args(&args)?;
    let listener = TcpListener::bind(&addr)?;
    println!("projector-server listening on {addr}");
    match store {
        StoreConfig::Sqlite(sqlite_path) => {
            projector_server::serve_sqlite(listener, sqlite_path).await?
        }
        StoreConfig::FileBacked(state_dir) => {
            projector_server::serve_file_backed(listener, state_dir).await?
        }
        StoreConfig::Postgres(postgres_url) => {
            projector_server::serve_postgres(listener, postgres_url).await?
        }
    }
    Ok(())
}

#[derive(Debug)]
enum StoreConfig {
    Sqlite(PathBuf),
    FileBacked(PathBuf),
    Postgres(String),
}

fn parse_serve_args(args: &[String]) -> Result<(String, StoreConfig), Box<dyn Error>> {
    let mut addr = None;
    let mut sqlite_path = None;
    let mut state_dir = None;
    let mut postgres_url = None;
    let mut idx = 0;
    while idx < args.len() {
        match args[idx].as_str() {
            "--addr" => {
                idx += 1;
                addr = args.get(idx).cloned();
            }
            "--state-dir" => {
                idx += 1;
                state_dir = args.get(idx).map(PathBuf::from);
            }
            "--sqlite-path" => {
                idx += 1;
                sqlite_path = args.get(idx).map(PathBuf::from);
            }
            "--postgres-url" => {
                idx += 1;
                postgres_url = args.get(idx).cloned();
            }
            other => return Err(format!("unknown serve argument: {other}").into()),
        }
        idx += 1;
    }

    let addr = addr.ok_or("missing --addr")?;
    let store = match (sqlite_path, state_dir, postgres_url) {
        (Some(sqlite_path), None, None) => StoreConfig::Sqlite(sqlite_path),
        (None, Some(state_dir), None) => StoreConfig::FileBacked(state_dir),
        (None, None, Some(postgres_url)) => StoreConfig::Postgres(postgres_url),
        (None, None, None) => {
            return Err("missing --sqlite-path, --state-dir, or --postgres-url".into());
        }
        _ => {
            return Err(
                "choose exactly one of --sqlite-path, --state-dir, or --postgres-url".into(),
            );
        }
    };
    Ok((addr, store))
}

fn print_usage() {
    println!(
        "Usage: projector-server serve --addr <host:port> (--sqlite-path <path> | --state-dir <path> | --postgres-url <url>)"
    );
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{StoreConfig, parse_serve_args};

    #[test]
    fn parse_serve_args_accepts_sqlite_backend() {
        let args = vec![
            "--addr".to_owned(),
            "127.0.0.1:9010".to_owned(),
            "--sqlite-path".to_owned(),
            "/tmp/projector.sqlite3".to_owned(),
        ];
        let (addr, store) = parse_serve_args(&args).expect("parse serve args");
        assert_eq!(addr, "127.0.0.1:9010");
        match store {
            StoreConfig::Sqlite(path) => {
                assert_eq!(path, PathBuf::from("/tmp/projector.sqlite3"));
            }
            other => panic!("expected sqlite backend, got {:?}", backend_name(&other)),
        }
    }

    #[test]
    fn parse_serve_args_accepts_postgres_backend() {
        let args = vec![
            "--addr".to_owned(),
            "127.0.0.1:9010".to_owned(),
            "--postgres-url".to_owned(),
            "postgres://localhost/projector".to_owned(),
        ];
        let (addr, store) = parse_serve_args(&args).expect("parse serve args");
        assert_eq!(addr, "127.0.0.1:9010");
        match store {
            StoreConfig::Postgres(url) => {
                assert_eq!(url, "postgres://localhost/projector");
            }
            other => panic!("expected postgres backend, got {:?}", backend_name(&other)),
        }
    }

    #[test]
    fn parse_serve_args_rejects_multiple_backends() {
        let args = vec![
            "--addr".to_owned(),
            "127.0.0.1:9010".to_owned(),
            "--sqlite-path".to_owned(),
            "/tmp/projector.sqlite3".to_owned(),
            "--postgres-url".to_owned(),
            "postgres://localhost/projector".to_owned(),
        ];
        let error = parse_serve_args(&args).expect_err("multiple backends should fail");
        assert!(
            error
                .to_string()
                .contains("choose exactly one of --sqlite-path, --state-dir, or --postgres-url")
        );
    }

    fn backend_name(store: &StoreConfig) -> &'static str {
        match store {
            StoreConfig::Sqlite(_) => "sqlite",
            StoreConfig::FileBacked(_) => "file",
            StoreConfig::Postgres(_) => "postgres",
        }
    }
}
