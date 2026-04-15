use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use projector_runtime::{FileServerProfileStore, ProjectorHome};

fn temp_dir(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time before unix epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("projector-{name}-{unique}"));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create temp dir");
    root
}

fn run_projector_with_env(repo_root: &Path, args: &[&str], envs: &[(&str, &str)]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_projector"))
        .args(args)
        .current_dir(repo_root)
        .envs(envs.iter().copied())
        .output()
        .expect("run projector");
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf8 stdout")
}

fn projector_server_bin() -> String {
    let projector_bin = PathBuf::from(env!("CARGO_BIN_EXE_projector"));
    projector_bin
        .parent()
        .expect("projector bin dir")
        .join("projector-server")
        .display()
        .to_string()
}

struct DockerSshServer {
    container_id: String,
    ssh_port: String,
    app_port: String,
}

impl DockerSshServer {
    fn start(public_key: &str) -> Self {
        let output = Command::new("docker")
            .args([
                "run",
                "-d",
                "--rm",
                "-e",
                &format!("PUBLIC_KEY={public_key}"),
                "-e",
                "PASSWORD_ACCESS=false",
                "-e",
                "USER_NAME=projector",
                "-p",
                "0:2222",
                "-p",
                "0:8942",
                "lscr.io/linuxserver/openssh-server:latest",
            ])
            .output()
            .expect("start openssh container");
        assert!(
            output.status.success(),
            "docker run failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let container_id = String::from_utf8(output.stdout)
            .expect("utf8 container id")
            .trim()
            .to_owned();
        let ssh_port = wait_for_docker_port(&container_id, "2222/tcp");
        let app_port = wait_for_docker_port(&container_id, "8942/tcp");
        Self {
            container_id,
            ssh_port,
            app_port,
        }
    }
}

impl Drop for DockerSshServer {
    fn drop(&mut self) {
        let _ = Command::new("docker")
            .args(["rm", "-f", &self.container_id])
            .status();
    }
}

fn wait_for_docker_port(container_id: &str, port: &str) -> String {
    for _ in 0..120 {
        let output = Command::new("docker")
            .args(["port", container_id, port])
            .output()
            .expect("inspect docker port");
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
    panic!("container did not publish host port for {port}");
}

fn generate_ssh_identity(home: &Path) -> PathBuf {
    let ssh_dir = home.join(".ssh");
    fs::create_dir_all(&ssh_dir).expect("create ssh dir");
    let key_path = ssh_dir.join("id_ed25519");
    let output = Command::new("ssh-keygen")
        .args(["-t", "ed25519", "-N", "", "-f"])
        .arg(&key_path)
        .args(["-C", "projector-test"])
        .output()
        .expect("run ssh-keygen");
    assert!(
        output.status.success(),
        "ssh-keygen failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    key_path
}

fn write_ssh_config(home: &Path, key_path: &Path, ssh_port: &str) {
    let config_path = home.join(".ssh/config");
    fs::write(
        &config_path,
        format!(
            "Host deploy-box\n  HostName 127.0.0.1\n  Port {ssh_port}\n  User projector\n  IdentityFile {}\n  StrictHostKeyChecking no\n  UserKnownHostsFile /dev/null\n",
            key_path.display()
        ),
    )
    .expect("write ssh config");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&config_path, fs::Permissions::from_mode(0o600))
            .expect("chmod ssh config");
    }
}

fn wait_for_ssh_ready(home: &Path) {
    for _ in 0..120 {
        let output = Command::new("ssh")
            .env("HOME", home)
            .args(["deploy-box", "true"])
            .output()
            .expect("check ssh readiness");
        if output.status.success() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    panic!("docker ssh server did not become ready");
}

#[test]
#[ignore = "requires local docker with ssh image availability"]
fn deploy_ssh_e2e_copies_binary_into_docker_and_registers_profile() {
    let workspace = temp_dir("deploy-e2e");
    let projector_home = temp_dir("deploy-e2e-home");
    let key_path = generate_ssh_identity(&projector_home);
    let public_key =
        fs::read_to_string(key_path.with_extension("pub")).expect("read generated public key");
    let ssh_server = DockerSshServer::start(public_key.trim());
    write_ssh_config(&projector_home, &key_path, &ssh_server.ssh_port);
    wait_for_ssh_ready(&projector_home);

    let projector_home_str = projector_home.to_str().expect("projector home utf8");
    let home_str = projector_home.to_str().expect("home utf8");
    let server_bin = projector_server_bin();
    let server_addr = format!("127.0.0.1:{}", ssh_server.app_port);

    let output = run_projector_with_env(
        &workspace,
        &[
            "deploy",
            "--profile",
            "dockerbox",
            "--ssh",
            "deploy-box",
            "--server-addr",
            &server_addr,
            "--yes",
        ],
        &[
            ("PROJECTOR_HOME", projector_home_str),
            ("HOME", home_str),
            ("PROJECTOR_SERVER_BIN", &server_bin),
        ],
    );

    assert!(output.contains("deploy: complete"));
    assert!(output.contains("backend: sqlite"));
    assert!(output.contains("profile: dockerbox"));

    let test_binary = Command::new("ssh")
        .env("HOME", &projector_home)
        .args(["deploy-box", "test", "-x", "~/.projector/projector-server"])
        .status()
        .expect("check remote binary");
    assert!(
        test_binary.success(),
        "remote projector-server binary missing"
    );

    let registry = FileServerProfileStore::new(ProjectorHome::new(&projector_home))
        .load()
        .expect("load profile registry");
    assert_eq!(registry.profiles.len(), 1);
    assert_eq!(registry.profiles[0].profile_id, "dockerbox");
    assert_eq!(registry.profiles[0].server_addr, server_addr);
}
