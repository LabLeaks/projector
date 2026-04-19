use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_dir(prefix: &str, name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("{prefix}-{name}-{unique}"));
    std::fs::create_dir_all(&path).expect("create temp test directory");
    path
}

pub(crate) fn temp_projector_home(name: &str) -> PathBuf {
    temp_dir("projector-home", name)
}

pub(crate) fn temp_repo_root(name: &str) -> PathBuf {
    temp_dir("projector", name)
}
