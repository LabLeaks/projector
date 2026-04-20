#![allow(dead_code)]
/**
@module PROJECTOR.TESTS.SUPPORT.QUALITY
Release-review test helpers in `tests/support/quality.rs`.
*/
// @fileimplements PROJECTOR.TESTS.SUPPORT.QUALITY
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("cli dir parent")
        .parent()
        .expect("repo root")
        .to_path_buf()
}

pub fn review_script_path() -> PathBuf {
    repo_root().join("scripts/review-projector-release-style.py")
}

pub fn workflow_files() -> Vec<PathBuf> {
    fs::read_dir(repo_root().join(".github/workflows"))
        .expect("workflow directory should be readable")
        .map(|entry| entry.expect("workflow entry should be readable").path())
        .filter(|path| {
            matches!(
                path.extension().and_then(|ext| ext.to_str()),
                Some("yml" | "yaml")
            )
        })
        .collect()
}

pub fn release_review_schema() -> Value {
    serde_json::from_str(
        &fs::read_to_string(repo_root().join("scripts/projector-release-review.schema.json"))
            .expect("release review schema should be readable"),
    )
    .expect("release review schema should be valid json")
}

pub fn release_review_dry_run(args: &[&str]) -> Value {
    let output = Command::new("python3")
        .arg("scripts/review-projector-release-style.py")
        .args(args)
        .arg("--dry-run")
        .current_dir(repo_root())
        .output()
        .expect("release review dry-run should run");
    assert!(
        output.status.success(),
        "stdout:\n{}\n\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("dry-run output should be valid json")
}

pub fn release_review_python_helper(script: &str, args: &[&str]) -> Value {
    let output = Command::new("python3")
        .arg("-c")
        .arg(script)
        .arg(repo_root())
        .args(args)
        .current_dir(repo_root())
        .output()
        .expect("release review helper should run");
    assert!(
        output.status.success(),
        "stdout:\n{}\n\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("helper output should be valid json")
}

fn release_review_extract_context_ranges_with_mode(
    path: &str,
    content: &str,
    start: i64,
    end: i64,
    full_scan: bool,
) -> Value {
    let script = r#"
import importlib.util
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
path = sys.argv[2]
content = sys.argv[3]
start = int(sys.argv[4])
end = int(sys.argv[5])
full_scan = sys.argv[6] == "true"
spec = importlib.util.spec_from_file_location(
    "release_review", root / "scripts" / "review-projector-release-style.py"
)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
print(json.dumps(module.extract_context_ranges(path, content, [(start, end)], full_scan)))
"#;

    release_review_python_helper(
        script,
        &[
            path,
            content,
            &start.to_string(),
            &end.to_string(),
            if full_scan { "true" } else { "false" },
        ],
    )
}

pub fn release_review_extract_context_ranges(
    path: &str,
    content: &str,
    start: i64,
    end: i64,
) -> Value {
    release_review_extract_context_ranges_with_mode(path, content, start, end, false)
}

pub fn release_review_extract_full_scan_context_ranges(path: &str, content: &str) -> Value {
    release_review_extract_context_ranges_with_mode(path, content, 1, 1, true)
}

pub fn release_review_chunk_helper(context_chars: usize) -> Value {
    let script = r#"
import importlib.util
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
context_chars = int(sys.argv[2])
spec = importlib.util.spec_from_file_location(
    "release_review", root / "scripts" / "review-projector-release-style.py"
)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
review_pass = {"name": "test", "focus": ["budgeting"], "files": ["src/example.rs"]}
contexts = [
    {
        "path": "src/example.rs",
        "start_line": 1,
        "end_line": 1,
        "content": "x" * context_chars,
    },
    {
        "path": "src/example.rs",
        "start_line": 2,
        "end_line": 2,
        "content": "y" * context_chars,
    },
]
chunks, runner_warnings = module.build_pass_chunks(
    root,
    sys.argv[3],
    "jj",
    None,
    "@",
    True,
    review_pass,
    contexts,
)
print(json.dumps({"chunks": chunks, "runner_warnings": runner_warnings}))
"#;

    let version = current_package_version();
    release_review_python_helper(script, &[&context_chars.to_string(), &version])
}

pub fn release_review_changed_line_ranges(diff: &str) -> Value {
    let script = r#"
import importlib.util
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
diff = sys.argv[2]
spec = importlib.util.spec_from_file_location(
    "release_review", root / "scripts" / "review-projector-release-style.py"
)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
print(json.dumps(module.parse_changed_line_ranges(diff)))
"#;

    release_review_python_helper(script, &[diff])
}

pub fn release_review_passes_for(files: &[&str]) -> Value {
    let script = r#"
import importlib.util
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
files = sys.argv[2:]
spec = importlib.util.spec_from_file_location(
    "release_review", root / "scripts" / "review-projector-release-style.py"
)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
print(json.dumps(module.build_review_passes(files)))
"#;

    release_review_python_helper(script, files)
}

pub fn release_review_merge_responses(responses: &Value) -> Value {
    let script = r#"
import importlib.util
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
responses = json.loads(sys.argv[2])
spec = importlib.util.spec_from_file_location(
    "release_review", root / "scripts" / "review-projector-release-style.py"
)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
print(json.dumps(module.merge_pass_responses("v0.2.0", False, responses, [])))
"#;

    let responses_json = serde_json::to_string(responses).expect("responses should serialize");
    release_review_python_helper(script, &[&responses_json])
}

fn release_review_validate_response_shape_output(payload: &str) -> std::process::Output {
    let script = r#"
import importlib.util
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
payload = json.loads(sys.argv[2])
spec = importlib.util.spec_from_file_location(
    "release_review", root / "scripts" / "review-projector-release-style.py"
)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
try:
    validated = module.validate_response_shape(payload)
    print(json.dumps(validated))
except SystemExit as err:
    print(str(err), file=sys.stderr)
    raise
"#;

    Command::new("python3")
        .arg("-c")
        .arg(script)
        .arg(repo_root())
        .arg(payload)
        .current_dir(repo_root())
        .output()
        .expect("response validation helper should run")
}

pub fn release_review_validate_response_shape_ok(payload: &str) -> Value {
    let output = release_review_validate_response_shape_output(payload);
    assert!(
        output.status.success(),
        "stdout:\n{}\n\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("validated response should be valid json")
}

pub fn release_review_validate_response_shape_err(payload: &str) -> String {
    let output = release_review_validate_response_shape_output(payload);
    assert!(
        !output.status.success(),
        "stdout:\n{}\n\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stderr).expect("stderr should be utf-8")
}

pub fn release_review_validate_response_shape_nonjson_value_err(payload: &str) -> String {
    let script = r#"
import importlib.util
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
payload = json.loads(sys.argv[2])
spec = importlib.util.spec_from_file_location(
    "release_review", root / "scripts" / "review-projector-release-style.py"
)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
try:
    validated = module.validate_response_shape(payload)
    print(json.dumps(validated))
except SystemExit as err:
    print(str(err), file=sys.stderr)
    raise
"#;

    let output = Command::new("python3")
        .arg("-c")
        .arg(script)
        .arg(repo_root())
        .arg(payload)
        .current_dir(repo_root())
        .output()
        .expect("non-object response validation helper should run");
    assert!(
        !output.status.success(),
        "stdout:\n{}\n\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stderr).expect("stderr should be utf-8")
}

pub fn python_entrypoint_runtime_flag(script_name: &str) -> Value {
    let script_path = repo_root().join("scripts").join(script_name);
    let script = r#"
import importlib.util
import json
import pathlib
import sys

script_path = pathlib.Path(sys.argv[1])
spec = importlib.util.spec_from_file_location("entrypoint", script_path)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
print(json.dumps({"dont_write_bytecode": sys.dont_write_bytecode}))
"#;
    let script_arg = script_path.to_string_lossy().into_owned();
    let output = Command::new("python3")
        .arg("-c")
        .arg(script)
        .arg(script_arg)
        .current_dir(repo_root())
        .output()
        .expect("entrypoint runtime flag helper should run");
    assert!(
        output.status.success(),
        "stdout:\n{}\n\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout)
        .expect("entrypoint runtime flag output should be valid json")
}

pub fn latest_reachable_semver_tag() -> String {
    let output = Command::new("jj")
        .args(["--ignore-working-copy", "tag", "list", "-r", "::@-"])
        .current_dir(repo_root())
        .output()
        .expect("jj tag list should run");
    assert!(
        output.status.success(),
        "stdout:\n{}\n\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let mut versions = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Some((tag, _)) = line.split_once(':') else {
            continue;
        };
        let tag = tag.trim();
        let Some(version) = parse_semver_sort_key(tag) else {
            continue;
        };
        versions.push((version, tag.to_string()));
    }

    versions.sort();
    versions
        .last()
        .map(|(_, tag)| tag.clone())
        .expect("repo should have at least one semver tag")
}

type SemverSortKey = (u64, u64, u64, u8, Vec<(u8, String)>);

fn parse_semver_sort_key(tag: &str) -> Option<SemverSortKey> {
    let stripped = tag.strip_prefix('v').unwrap_or(tag);
    let (core, prerelease) = match stripped.split_once('-') {
        Some((core, prerelease)) => (core, Some(prerelease)),
        None => (stripped, None),
    };
    let mut parts = core.split('.');
    let (Some(major), Some(minor), Some(patch), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    let (Ok(major), Ok(minor), Ok(patch)) = (
        major.parse::<u64>(),
        minor.parse::<u64>(),
        patch.parse::<u64>(),
    ) else {
        return None;
    };
    let prerelease_key = prerelease
        .map(|value| {
            value
                .split('.')
                .map(|part| {
                    if part.chars().all(|ch| ch.is_ascii_digit()) {
                        (0, format!("{:020}:{}", part.len(), part))
                    } else {
                        (1, part.to_string())
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    Some((
        major,
        minor,
        patch,
        if prerelease.is_none() { 1 } else { 0 },
        prerelease_key,
    ))
}

fn current_package_version() -> String {
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
        .expect("workspace version should be utf-8")
        .trim()
        .to_string()
}
