/**
@module PROJECTOR.EDGE.DEPLOY_CLI
Owns SQLite-first remote BYO deployment over SSH and sysbox, including remote build/package orchestration and resulting profile registration.
*/
// @fileimplements PROJECTOR.EDGE.DEPLOY_CLI
use std::error::Error;
use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use projector_runtime::{FileServerProfileStore, ProjectorHome, discover_repo_root};

use crate::cli_support::now_ns;

use super::args::{DeployArgs, parse_deploy_args};
use super::profiles::wait_for_server_reachability;
use super::prompts::{fill_deploy_defaults, prompt_confirm};

pub(crate) fn run_deploy(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    let mut deploy_args = parse_deploy_args(&args)?;
    let interactive = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    deploy_args = fill_deploy_defaults(deploy_args, interactive)?;

    let plan = DeployPlan::from_args(&deploy_args)?;
    let source_root = discover_repo_root(&std::env::current_dir()?);
    let local_source_archive = create_deploy_source_archive(&source_root)?;
    if interactive && !deploy_args.yes {
        print_deploy_summary(&plan);
        if !prompt_confirm("Proceed with remote deploy? [Y/n]: ", true)? {
            println!("deploy: cancelled");
            let _ = fs::remove_file(&local_source_archive);
            return Ok(());
        }
    }

    let deploy_result = (|| {
        run_scp(&plan, &local_source_archive)?;
        run_ssh(&plan)
    })();
    let _ = fs::remove_file(&local_source_archive);
    deploy_result?;

    let home = ProjectorHome::discover()?;
    let profiles = FileServerProfileStore::new(home);
    profiles.upsert_profile(&plan.profile_id, &plan.server_addr, Some(&plan.ssh_target))?;

    println!("deploy: complete");
    println!("profile: {}", plan.profile_id);
    println!("server_addr: {}", plan.server_addr);
    println!("ssh_target: {}", plan.ssh_target);
    println!("backend: sqlite");
    println!("isolation: sysbox");
    println!("container: {}", plan.container_name);
    let reachable = wait_for_server_reachability(&plan.server_addr, 20, Duration::from_millis(250));
    println!("reachable: {}", reachable);
    if !reachable {
        println!(
            "deploy_warning: {} is not reachable yet; verify network path and container startup",
            plan.server_addr
        );
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub(super) struct DeployPlan {
    profile_id: String,
    ssh_target: String,
    server_addr: String,
    remote_dir: String,
    sqlite_path: String,
    listen_addr: String,
    remote_bin_path: String,
    remote_source_archive_path: String,
    remote_build_dir: String,
    container_name: String,
    builder_image: String,
    container_image: String,
    container_mount_dir: String,
    container_sqlite_path: String,
    publish_spec: String,
}

impl DeployPlan {
    fn from_args(args: &DeployArgs) -> Result<Self, Box<dyn Error>> {
        let profile_id = args.profile_id.clone().ok_or("missing deploy profile id")?;
        let ssh_target = args.ssh_target.clone().ok_or("missing deploy ssh target")?;
        let server_addr = args
            .server_addr
            .clone()
            .ok_or("missing deploy server addr")?;
        let remote_dir = args.remote_dir.clone().ok_or("missing deploy remote dir")?;
        let sqlite_path = args
            .sqlite_path
            .clone()
            .ok_or("missing deploy sqlite path")?;
        let listen_addr = args
            .listen_addr
            .clone()
            .ok_or("missing deploy listen addr")?;
        let remote_dir_trimmed = remote_dir.trim_end_matches('/').to_owned();
        let remote_bin_path = format!("{remote_dir_trimmed}/projector-server");
        let remote_source_archive_path = format!("{remote_dir_trimmed}/projector-source.tar.gz");
        let remote_build_dir = format!("{remote_dir_trimmed}/build-{}", now_ns());
        let builder_image = "rust:1.87-bookworm".to_owned();
        let container_name = format!("projector-{}", sanitize_container_name(&profile_id));
        let container_image = "debian:bookworm-slim".to_owned();
        let container_mount_dir = "/srv/projector".to_owned();
        let container_sqlite_path = format!(
            "{}/{}",
            container_mount_dir,
            sqlite_path
                .rsplit('/')
                .next()
                .filter(|value| !value.is_empty())
                .ok_or("sqlite path must include a filename")?
        );
        let publish_spec = docker_publish_spec(&listen_addr)?;

        Ok(Self {
            profile_id,
            ssh_target,
            server_addr,
            remote_dir,
            sqlite_path,
            listen_addr,
            remote_bin_path,
            remote_source_archive_path,
            remote_build_dir,
            container_name,
            builder_image,
            container_image,
            container_mount_dir,
            container_sqlite_path,
            publish_spec,
        })
    }
}

pub(super) fn infer_server_addr(
    ssh_target: &str,
    listen_addr: &str,
) -> Result<String, Box<dyn Error>> {
    let host = ssh_target
        .rsplit('@')
        .next()
        .filter(|value| !value.is_empty())
        .ok_or("ssh target must include a host")?;
    let port = listen_addr
        .rsplit(':')
        .next()
        .ok_or("listen address must include a port")?;
    Ok(format!("{host}:{port}"))
}

fn print_deploy_summary(plan: &DeployPlan) {
    println!("deploy_profile: {}", plan.profile_id);
    println!("deploy_ssh_target: {}", plan.ssh_target);
    println!("deploy_server_addr: {}", plan.server_addr);
    println!("deploy_backend: sqlite");
    println!("deploy_isolation: sysbox");
    println!("deploy_remote_dir: {}", plan.remote_dir);
    println!("deploy_sqlite_path: {}", plan.sqlite_path);
    println!("deploy_listen_addr: {}", plan.listen_addr);
    println!("deploy_builder_image: {}", plan.builder_image);
    println!("deploy_container: {}", plan.container_name);
    println!("deploy_image: {}", plan.container_image);
}

fn run_scp(plan: &DeployPlan, local_source_archive: &Path) -> Result<(), Box<dyn Error>> {
    let mkdir_output = Command::new("ssh")
        .arg(&plan.ssh_target)
        .arg(format!(
            "mkdir -p {}",
            shell_quote_remote_path(&plan.remote_dir)
        ))
        .output()?;
    if !mkdir_output.status.success() {
        return Err(format!(
            "ssh precreate remote dir failed: {}",
            String::from_utf8_lossy(&mkdir_output.stderr)
        )
        .into());
    }

    let destination = format!("{}:{}", plan.ssh_target, plan.remote_source_archive_path);
    let output = Command::new("scp")
        .arg(local_source_archive)
        .arg(&destination)
        .output()?;
    if !output.status.success() {
        return Err(format!("scp failed: {}", String::from_utf8_lossy(&output.stderr)).into());
    }
    Ok(())
}

fn run_ssh(plan: &DeployPlan) -> Result<(), Box<dyn Error>> {
    let output = Command::new("ssh")
        .arg(&plan.ssh_target)
        .arg(build_remote_deploy_script(plan))
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "ssh deploy failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(())
}

fn build_remote_deploy_script(plan: &DeployPlan) -> String {
    format!(
        "set -e; \
command -v docker >/dev/null 2>&1 || {{ echo 'docker is required on remote host' >&2; exit 1; }}; \
docker info --format '{{{{json .Runtimes}}}}' | grep -q 'sysbox-runc' || {{ echo 'sysbox-runc runtime is required on remote host' >&2; exit 1; }}; \
mkdir -p {remote_dir} {remote_build_dir}; \
tar -xzf {source_archive} -C {remote_build_dir}; \
docker run --rm --user \"$(id -u):$(id -g)\" -v {remote_build_dir}:/work -w /work {builder_image} bash -lc '/usr/local/cargo/bin/cargo build --release -p projector-server' >/dev/null; \
if docker ps -a --format '{{{{.Names}}}}' | grep -Fxq {container}; then docker rm -f {container} >/dev/null 2>&1 || true; fi; \
cp {built_bin} {tmp_bin}; chmod +x {tmp_bin}; mv {tmp_bin} {bin}; \
docker pull {image} >/dev/null; \
docker run -d --name {container} --restart unless-stopped --runtime=sysbox-runc -p {publish_spec} \
-v {remote_dir}:{container_mount_dir} {image} {container_bin} serve --addr {listen_addr} --sqlite-path {container_sqlite_path} >/dev/null; \
rm -rf {remote_build_dir} >/dev/null 2>&1 || true",
        remote_dir = shell_quote_remote_path(&plan.remote_dir),
        remote_build_dir = shell_quote_remote_path(&plan.remote_build_dir),
        source_archive = shell_quote_remote_path(&plan.remote_source_archive_path),
        builder_image = shell_quote(&plan.builder_image),
        built_bin = shell_quote_remote_path(&format!(
            "{}/target/release/projector-server",
            plan.remote_build_dir
        )),
        bin = shell_quote_remote_path(&plan.remote_bin_path),
        tmp_bin = shell_quote_remote_path(&format!("{}.tmp", plan.remote_bin_path)),
        image = shell_quote(&plan.container_image),
        container = shell_quote(&plan.container_name),
        publish_spec = shell_quote(&plan.publish_spec),
        container_mount_dir = shell_quote(&plan.container_mount_dir),
        container_bin = shell_quote(&format!("{}/projector-server", plan.container_mount_dir)),
        listen_addr = shell_quote(&plan.listen_addr),
        container_sqlite_path = shell_quote(&plan.container_sqlite_path),
    )
}

fn sanitize_container_name(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '_' | '.' | '-' => ch,
            _ => '-',
        })
        .collect::<String>()
        .trim_matches('-')
        .to_ascii_lowercase()
}

fn docker_publish_spec(listen_addr: &str) -> Result<String, Box<dyn Error>> {
    let mut parts = listen_addr.rsplitn(2, ':');
    let port = parts
        .next()
        .filter(|value| !value.is_empty())
        .ok_or("listen address must include a port")?;
    let host = parts
        .next()
        .filter(|value| !value.is_empty())
        .ok_or("listen address must include a host")?;
    Ok(format!("{host}:{port}:{port}"))
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn shell_quote_remote_path(value: &str) -> String {
    if let Some(rest) = value.strip_prefix("~/") {
        return format!("\"$HOME/{}\"", rest.replace('"', "\\\""));
    }
    if let Some(rest) = value.strip_prefix("$HOME/") {
        return format!("\"$HOME/{}\"", rest.replace('"', "\\\""));
    }
    shell_quote(value)
}

fn create_deploy_source_archive(source_root: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let archive_path = std::env::temp_dir().join(format!("projector-deploy-{}.tar.gz", now_ns()));
    let output = Command::new("tar")
        .env("COPYFILE_DISABLE", "1")
        .arg("-czf")
        .arg(&archive_path)
        .arg("--exclude")
        .arg(".git")
        .arg("--exclude")
        .arg("target")
        .arg("-C")
        .arg(source_root)
        .arg(".")
        .output()?;
    if !output.status.success() {
        let _ = fs::remove_file(&archive_path);
        return Err(format!(
            "failed to create deploy source archive: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(archive_path)
}
