use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

static RELEASE_MOCK_LOG_COUNTER: AtomicU64 = AtomicU64::new(0);
static GH_MOCK_COUNTER: AtomicU64 = AtomicU64::new(0);

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("cli dir parent")
        .parent()
        .expect("repo root")
        .to_path_buf()
}

fn current_workspace_version() -> String {
    let output = Command::new("python3")
        .arg("-c")
        .arg(
            r#"
from pathlib import Path
import sys
sys.path.insert(0, str((Path.cwd() / "scripts").resolve()))
from release_tooling import package_version
print(package_version(Path.cwd()))
"#,
        )
        .current_dir(repo_root())
        .output()
        .expect("workspace version probe should run");
    assert!(
        output.status.success(),
        "stdout:\n{}\n\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("utf8 workspace version")
        .trim()
        .to_string()
}

fn current_python_executable() -> String {
    let output = Command::new("python3")
        .arg("-c")
        .arg("import sys; print(sys.executable)")
        .current_dir(repo_root())
        .output()
        .expect("python executable probe should run");
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .expect("python executable should be utf-8")
        .trim()
        .to_string()
}

fn current_release_revision() -> String {
    let output = Command::new("jj")
        .args([
            "--ignore-working-copy",
            "log",
            "-r",
            "@-",
            "--no-graph",
            "-T",
            "commit_id",
        ])
        .current_dir(repo_root())
        .output()
        .expect("jj log should run");
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .expect("utf8 revision")
        .trim()
        .to_string()
}

fn tag_points_at_current_revision(tag: &str) -> bool {
    let output = Command::new("jj")
        .args([
            "--ignore-working-copy",
            "log",
            "-r",
            tag,
            "--no-graph",
            "-T",
            "commit_id",
        ])
        .current_dir(repo_root())
        .output()
        .expect("jj tag lookup should run");
    if !output.status.success() {
        return false;
    }
    String::from_utf8(output.stdout)
        .expect("utf8 tag revision")
        .trim()
        == current_release_revision()
}

fn tag_exists(tag: &str) -> bool {
    let output = Command::new("jj")
        .args(["--ignore-working-copy", "log", "-r", tag, "--no-graph", "-T", "commit_id"])
        .current_dir(repo_root())
        .output()
        .expect("jj tag lookup should run");
    output.status.success()
}

fn effective_release_args(version: &str, args: &[&str]) -> Vec<String> {
    let mut effective = args.iter().map(|arg| (*arg).to_string()).collect::<Vec<_>>();
    let tag = format!("v{version}");
    if tag_exists(&tag)
        && !effective.iter().any(|arg| arg == "--allow-existing-tag")
    {
        effective.push("--allow-existing-tag".to_string());
    }
    effective
}

fn release_tag_dry_run(version: &str, args: &[&str]) -> Value {
    let args = effective_release_args(version, args);
    let output = Command::new("python3")
        .arg("scripts/tag-release.py")
        .arg(version)
        .args(&args)
        .arg("--dry-run")
        .current_dir(repo_root())
        .output()
        .expect("release tag dry-run should run");
    assert!(
        output.status.success(),
        "stdout:\n{}\n\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("dry-run output should be valid json")
}

struct ReleaseExecution {
    output: Output,
    mock_log: Vec<Value>,
}

fn release_tag_live_output(version: &str, args: &[&str]) -> ReleaseExecution {
    release_tag_live_output_with_input(version, args, "")
}

fn release_tag_live_output_with_input(
    version: &str,
    args: &[&str],
    input: &str,
) -> ReleaseExecution {
    let args = effective_release_args(version, args);
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time before unix epoch")
        .as_nanos();
    let counter = RELEASE_MOCK_LOG_COUNTER.fetch_add(1, Ordering::Relaxed);
    let log_path =
        std::env::temp_dir().join(format!("projector-release-mock-{unique}-{counter}.jsonl"));

    let mut command = Command::new("python3");
    command
        .arg("scripts/tag-release.py")
        .arg(version)
        .args(&args)
        .arg("--allow-mock-publish")
        .current_dir(repo_root())
        .env("PROJECTOR_RELEASE_MOCK_LOG_PATH", &log_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("release tag script should run");
    if !input.is_empty() {
        child
            .stdin
            .as_mut()
            .expect("stdin should be piped")
            .write_all(input.as_bytes())
            .expect("mock input should be written");
    }
    drop(child.stdin.take());
    let output = child
        .wait_with_output()
        .expect("release tag output should be captured");

    let mock_log = if log_path.exists() {
        let lines = fs::read_to_string(&log_path).expect("release mock log should be readable");
        let parsed = lines
            .lines()
            .map(|line| serde_json::from_str(line).expect("mock log entry should be json"))
            .collect();
        let _ = fs::remove_file(&log_path);
        parsed
    } else {
        Vec::new()
    };

    ReleaseExecution { output, mock_log }
}

fn base64_encode(input: &str) -> String {
    let mut child = Command::new("python3")
        .args([
            "-c",
            "import base64,sys; print(base64.b64encode(sys.stdin.buffer.read()).decode())",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("python3 should run");
    child
        .stdin
        .as_mut()
        .expect("stdin should be piped")
        .write_all(input.as_bytes())
        .expect("input should be written");
    let output = child
        .wait_with_output()
        .expect("python output should be captured");
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .expect("base64 output should be utf-8")
        .trim()
        .to_string()
}

fn release_assets() -> Value {
    serde_json::from_str(include_str!("../../../scripts/release-assets.json"))
        .expect("release assets json should be valid")
}

fn valid_release_assets_json() -> String {
    let assets = release_assets()["github_release_assets"]
        .as_array()
        .expect("github assets should be an array")
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let name = name.as_str().expect("asset name should be string");
            let mut asset = serde_json::json!({ "name": name });
            if name != "dist-manifest.json" {
                asset["digest"] = Value::String(format!("sha256:{:064x}", index + 1));
            }
            asset
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "tagName": format!("v{}", current_workspace_version()),
        "isDraft": false,
        "isPrerelease": false,
        "assets": assets
    })
    .to_string()
}

fn valid_formula_for_release(version: &str) -> String {
    format!(
        r#"# typed: false
# frozen_string_literal: true

class Projector < Formula
  version "{version}"
  archive = on_system_conditional(
    macos: on_arch_conditional(
      arm: "projector-cli-aarch64-apple-darwin.tar.xz",
      intel: "projector-cli-x86_64-apple-darwin.tar.xz"
    ),
    linux: on_arch_conditional(
      arm: "projector-cli-aarch64-unknown-linux-gnu.tar.xz",
      intel: "projector-cli-x86_64-unknown-linux-gnu.tar.xz"
    )
  )
  url "https://github.com/LabLeaks/projector/releases/download/v{version}/#{{archive}}"
  sha256 on_system_conditional(
    macos: on_arch_conditional(
      arm: "{sha_macos_arm}",
      intel: "{sha_macos_intel}"
    ),
    linux: on_arch_conditional(
      arm: "{sha_linux_arm}",
      intel: "{sha_linux_intel}"
    )
  )

  def install
    bin.install "projector"
  end
end
"#,
        sha_macos_arm = format!("{:064x}", 2),
        sha_linux_arm = format!("{:064x}", 4),
        sha_macos_intel = format!("{:064x}", 6),
        sha_linux_intel = format!("{:064x}", 10),
    )
}

fn run_script_with_fake_gh(script_path: &str, release_json: &str, formula: Option<&str>) -> Output {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time before unix epoch")
        .as_nanos();
    let counter = GH_MOCK_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp = std::env::temp_dir().join(format!("projector-gh-mock-{unique}-{counter}"));
    fs::create_dir_all(&temp).expect("create gh mock dir");
    let bin_dir = temp.join("bin");
    fs::create_dir_all(&bin_dir).expect("create gh bin dir");

    let encoded_formula = formula.map(base64_encode).unwrap_or_default();

    fs::write(
        bin_dir.join("gh"),
        format!(
            r#"#!/bin/sh
if [ "$1" = "release" ] && [ "$2" = "view" ]; then
cat <<'EOF'
{release_json}
EOF
exit 0
fi
if [ "$1" = "api" ]; then
case " $* " in
  *" --jq .content "*)
    printf '%s' '{encoded_formula}'
    ;;
  *)
    echo '{{"content":"{encoded_formula}","sha":"mock-sha"}}'
    ;;
esac
exit 0
fi
echo "unexpected gh invocation: $@" >&2
exit 1
"#,
            release_json = release_json,
            encoded_formula = encoded_formula,
        ),
    )
    .expect("write fake gh");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(bin_dir.join("gh"), fs::Permissions::from_mode(0o755))
            .expect("chmod fake gh");
    }

    let original_path = std::env::var("PATH").unwrap_or_default();
    let output = Command::new("bash")
        .arg(script_path)
        .current_dir(repo_root())
        .env("PATH", format!("{}:{}", bin_dir.display(), original_path))
        .output()
        .expect("script should run");
    fs::remove_dir_all(&temp).expect("remove gh mock dir");
    output
}

#[test]
// @verifies PROJECTOR.DISTRIBUTION.RELEASE_FLOW.DRY_RUN
fn release_tag_dry_run_lists_checklist_and_publication_commands() {
    let version = current_workspace_version();
    let payload = release_tag_dry_run(&version, &[]);
    let revision = current_release_revision();

    assert_eq!(payload["tag"], Value::String(format!("v{version}")));
    assert_eq!(payload["revision"], Value::String(revision.clone()));
    let checklist = payload["checklist"]
        .as_array()
        .expect("checklist should be an array");
    let checklist_ids: Vec<_> = checklist
        .iter()
        .map(|entry| entry["id"].as_str().expect("checklist id should be string"))
        .collect();
    assert_eq!(
        checklist_ids,
        vec!["readme", "changelog", "version", "validation"]
    );
    assert_eq!(
        payload["push_main_command"]
            .as_array()
            .expect("push_main_command should be an array"),
        &vec![
            Value::String("jj".to_string()),
            Value::String("git".to_string()),
            Value::String("push".to_string()),
            Value::String("--bookmark".to_string()),
            Value::String("main".to_string()),
        ]
    );
    assert_eq!(
        payload["push_tag_command"]
            .as_array()
            .expect("push_tag_command should be an array"),
        &(if tag_exists(&format!("v{version}")) {
            vec![
                Value::String("git".to_string()),
                Value::String("push".to_string()),
                Value::String("--force".to_string()),
                Value::String("origin".to_string()),
                Value::String(format!("refs/tags/v{version}")),
            ]
        } else {
            vec![
                Value::String("git".to_string()),
                Value::String("push".to_string()),
                Value::String("origin".to_string()),
                Value::String(format!("refs/tags/v{version}")),
            ]
        })
    );
    assert_eq!(
        payload["update_homebrew_formula_command"]
            .as_array()
            .expect("update_homebrew_formula_command should be an array"),
        &vec![
            Value::String(current_python_executable()),
            Value::String(
                repo_root()
                    .join("scripts/update-homebrew-formula.py")
                    .display()
                    .to_string(),
            ),
        ]
    );
}

#[test]
// @verifies PROJECTOR.DISTRIBUTION.RELEASE_FLOW.CHECKLIST
fn release_tag_script_aborts_when_checklist_answer_is_no() {
    let version = current_workspace_version();
    let execution = release_tag_live_output_with_input(&version, &[], "n\n");
    let output = execution.output;

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("aborted release publishing"),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(execution.mock_log.is_empty());
}

#[test]
// @verifies PROJECTOR.DISTRIBUTION.RELEASE_FLOW.CHECKLIST
fn release_tag_script_aborts_cleanly_when_checklist_has_no_input() {
    let version = current_workspace_version();
    let execution = release_tag_live_output(&version, &[]);
    let output = execution.output;

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("interactive release checklist is unavailable"),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(execution.mock_log.is_empty());
}

#[test]
// @verifies PROJECTOR.DISTRIBUTION.RELEASE_FLOW.PUSHES_MAIN_AND_TAG
fn release_tag_script_skip_checklist_runs_publication_steps() {
    let version = current_workspace_version();
    let execution = release_tag_live_output(&version, &["--skip-checklist"]);
    let output = execution.output;

    assert!(output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stdout).contains(&format!("Published release v{version}"))
    );
    let labels: Vec<_> = execution
        .mock_log
        .iter()
        .map(|entry| entry["label"].as_str().expect("label should be string"))
        .collect();
    let expected = if tag_exists(&format!("v{version}")) && tag_points_at_current_revision(&format!("v{version}")) {
        vec![
            "bookmark_main",
            "push_main",
            "push_tag",
            "verify_github_release",
            "update_homebrew_formula",
            "verify_homebrew_formula",
        ]
    } else {
        vec![
            "bookmark_main",
            "set_tag",
            "push_main",
            "push_tag",
            "verify_github_release",
            "update_homebrew_formula",
            "verify_homebrew_formula",
        ]
    };
    assert_eq!(labels, expected);
}

#[test]
// @verifies PROJECTOR.DISTRIBUTION.RELEASE_FLOW.VERIFIES_GITHUB_RELEASE
fn release_tag_script_runs_github_release_verification_step() {
    let version = current_workspace_version();
    let execution = release_tag_live_output(&version, &["--skip-checklist"]);
    let labels: Vec<_> = execution
        .mock_log
        .iter()
        .map(|entry| entry["label"].as_str().expect("label should be string"))
        .collect();
    assert!(labels.contains(&"verify_github_release"));
}

#[test]
// @verifies PROJECTOR.DISTRIBUTION.RELEASE_FLOW.UPDATES_HOMEBREW
fn release_tag_script_runs_homebrew_update_and_verification_steps() {
    let version = current_workspace_version();
    let execution = release_tag_live_output(&version, &["--skip-checklist"]);
    let labels: Vec<_> = execution
        .mock_log
        .iter()
        .map(|entry| entry["label"].as_str().expect("label should be string"))
        .collect();
    assert!(labels.contains(&"update_homebrew_formula"));
    assert!(labels.contains(&"verify_homebrew_formula"));
}

#[test]
// @verifies PROJECTOR.DISTRIBUTION.RELEASE_FLOW.MATCHES_WORKSPACE_VERSION
fn release_tag_script_requires_requested_tag_to_match_workspace_version() {
    let output = Command::new("python3")
        .arg("scripts/tag-release.py")
        .arg("9.9.9")
        .arg("--dry-run")
        .current_dir(repo_root())
        .output()
        .expect("release tag command should run");
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("does not match workspace version"),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
// @verifies PROJECTOR.DISTRIBUTION.GITHUB_RELEASES.WORKFLOW
fn github_release_workflow_is_committed() {
    let workflow = repo_root().join(".github/workflows/release.yml");
    assert!(workflow.is_file(), "release workflow should be committed");
    let content = fs::read_to_string(workflow).expect("workflow should be readable");
    assert!(content.contains("tags:\n      - '**[0-9]+.[0-9]+.[0-9]+*'"));
    assert!(content.contains("dist host"));
}

#[test]
// @verifies PROJECTOR.DISTRIBUTION.GITHUB_RELEASES.ASSET_MANIFEST
fn release_assets_manifest_covers_cli_server_and_homebrew_subset() {
    let assets = release_assets();
    let github_assets = assets["github_release_assets"]
        .as_array()
        .expect("github assets should be array");
    assert!(
        github_assets
            .iter()
            .any(|value| value.as_str() == Some("projector-cli-x86_64-apple-darwin.tar.xz"))
    );
    assert!(github_assets.iter().any(|value| value.as_str() == Some("projector-server-x86_64-unknown-linux-gnu.tar.xz")));

    let homebrew_assets = assets["homebrew_formula_archives"]
        .as_array()
        .expect("homebrew assets should be array");
    assert!(homebrew_assets.iter().all(|value| {
        value
            .as_str()
            .expect("homebrew asset name")
            .starts_with("projector-cli-")
    }));
}

#[test]
// @verifies PROJECTOR.DISTRIBUTION.RELEASE_FLOW.VERIFIES_GITHUB_RELEASE
fn github_release_verifier_accepts_expected_asset_set() {
    let output = run_script_with_fake_gh(
        "scripts/verify-github-release-published.sh",
        &valid_release_assets_json(),
        None,
    );
    assert!(
        output.status.success(),
        "stdout:\n{}\n\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
// @verifies PROJECTOR.DISTRIBUTION.HOMEBREW.FORMULA_AUTOMATION
fn homebrew_formula_verifier_accepts_expected_projector_formula_shape() {
    let version = current_workspace_version();
    let output = run_script_with_fake_gh(
        "scripts/verify-homebrew-formula.sh",
        &serde_json::json!({
            "assets": serde_json::from_str::<Value>(&valid_release_assets_json())
                .expect("release json should be valid")["assets"]
                .clone()
        })
        .to_string(),
        Some(&valid_formula_for_release(&version)),
    );
    assert!(
        output.status.success(),
        "stdout:\n{}\n\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
