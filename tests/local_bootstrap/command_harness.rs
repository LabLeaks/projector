/**
@module PROJECTOR.TESTS.SUPPORT.LOCAL_BOOTSTRAP_COMMAND_HARNESS
Repo-local command runners, PTY helpers, temporary environment setup, and server spawning for local-bootstrap proofs.
*/
// @fileimplements PROJECTOR.TESTS.SUPPORT.LOCAL_BOOTSTRAP_COMMAND_HARNESS
use super::*;

pub(crate) fn temp_repo(name: &str) -> PathBuf {
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

pub(crate) fn temp_projector_home(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time before unix epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("projector-home-{name}-{unique}"));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create temp projector home");
    root
}

pub(crate) fn run_projector(repo_root: &Path, args: &[&str]) -> String {
    run_projector_with_env(repo_root, args, &[])
}

pub(crate) fn run_projector_home(repo_root: &Path, projector_home: &Path, args: &[&str]) -> String {
    let projector_home_str = projector_home.to_str().expect("projector home utf8");
    run_projector_with_env(repo_root, args, &[("PROJECTOR_HOME", projector_home_str)])
}

pub(crate) fn run_projector_failure_with_env(
    repo_root: &Path,
    args: &[&str],
    envs: &[(&str, &str)],
) -> String {
    if is_legacy_sync_command(args) {
        return run_legacy_sync_with_env(repo_root, args, envs)
            .expect_err("legacy sync unexpectedly succeeded");
    }
    let merged_envs = merged_test_envs(repo_root, envs);
    let output = Command::new(env!("CARGO_BIN_EXE_projector"))
        .args(args)
        .current_dir(repo_root)
        .envs(merged_envs)
        .output()
        .expect("run projector");
    assert!(
        !output.status.success(),
        "command unexpectedly succeeded: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    String::from_utf8(output.stderr).expect("utf8 stderr")
}

pub(crate) fn run_projector_tty(repo_root: &Path, args: &[&str], input: &str) -> String {
    run_projector_tty_with_env(repo_root, args, input, &[])
}

pub(crate) fn run_projector_tty_with_env(
    repo_root: &Path,
    args: &[&str],
    input: &str,
    envs: &[(&str, &str)],
) -> String {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 40,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("open pty");
    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_projector"));
    cmd.cwd(repo_root);
    cmd.env("TERM", "xterm-256color");
    for (key, value) in merged_test_envs(repo_root, envs) {
        cmd.env(key, value);
    }
    for arg in args {
        cmd.arg(arg);
    }
    let mut child = pair
        .slave
        .spawn_command(cmd)
        .expect("spawn projector in pty");
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().expect("clone pty reader");
    let mut writer = pair.master.take_writer().expect("take pty writer");
    let output = Arc::new(Mutex::new(Vec::new()));
    let output_reader = Arc::clone(&output);
    let reader_thread = thread::spawn(move || {
        let mut buffer = Vec::new();
        reader.read_to_end(&mut buffer).expect("read pty output");
        *output_reader.lock().expect("lock output buffer") = buffer;
    });
    thread::sleep(Duration::from_millis(1000));
    writer.write_all(input.as_bytes()).expect("write pty input");
    writer.flush().expect("flush pty input");
    drop(writer);

    let status = child.wait().expect("wait for projector");
    reader_thread.join().expect("join pty reader");
    let output = String::from_utf8(output.lock().expect("lock output buffer").clone())
        .expect("utf8 pty output");
    assert!(status.success(), "tty command failed: {output}");
    output
}

pub(crate) fn install_fake_ssh_tools(root: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let bin_dir = root.join("fake-bin");
    fs::create_dir_all(&bin_dir).expect("create fake bin dir");
    let ssh_log = root.join("ssh.log");
    let scp_log = root.join("scp.log");

    fs::write(
        bin_dir.join("ssh"),
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"{}\"\nexit 0\n",
            ssh_log.display()
        ),
    )
    .expect("write fake ssh");
    fs::write(
        bin_dir.join("scp"),
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"{}\"\nexit 0\n",
            scp_log.display()
        ),
    )
    .expect("write fake scp");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(bin_dir.join("ssh"), fs::Permissions::from_mode(0o755))
            .expect("chmod fake ssh");
        fs::set_permissions(bin_dir.join("scp"), fs::Permissions::from_mode(0o755))
            .expect("chmod fake scp");
    }

    (bin_dir, ssh_log, scp_log)
}

pub(crate) fn spawn_server(state_dir: &Path) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind server addr");
    let addr = listener.local_addr().expect("local addr");
    projector_server::spawn_background(listener, state_dir.to_path_buf());
    std::thread::sleep(std::time::Duration::from_millis(150));
    addr
}

pub(crate) fn run_projector_with_env(
    repo_root: &Path,
    args: &[&str],
    envs: &[(&str, &str)],
) -> String {
    if is_legacy_sync_command(args) {
        return run_legacy_sync_with_env(repo_root, args, envs)
            .unwrap_or_else(|stderr| panic!("command failed: {stderr}"));
    }
    let merged_envs = merged_test_envs(repo_root, envs);
    let output = Command::new(env!("CARGO_BIN_EXE_projector"))
        .args(args)
        .current_dir(repo_root)
        .envs(merged_envs)
        .output()
        .expect("run projector");
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf8 stdout")
}

pub(crate) fn merged_test_envs(
    repo_root: &Path,
    envs: &[(&str, &str)],
) -> Vec<(String, String)> {
    let mut merged = vec![(
        "PROJECTOR_HOME".to_owned(),
        repo_root.join(".projector-test-home").display().to_string(),
    )];
    for (key, value) in envs {
        if *key == "PROJECTOR_HOME" {
            merged[0] = (key.to_string(), value.to_string());
        } else {
            merged.push((key.to_string(), value.to_string()));
        }
    }
    merged
}

fn is_legacy_sync_command(args: &[&str]) -> bool {
    args.first() == Some(&"sync") && !matches!(args.get(1), Some(&"start" | &"stop" | &"status"))
}
