/**
@group PROJECTOR.QUALITY
Repo-local quality contract surface for projector release tooling.

@group PROJECTOR.QUALITY.RELEASE_REVIEW
Local release-review contract surface.

@spec PROJECTOR.QUALITY.RELEASE_REVIEW.SPEC_OWNED
the release-review wrapper script carries the proving surface for the release-review contract.

@spec PROJECTOR.QUALITY.RELEASE_REVIEW.DEFAULT_MODEL
the default release-review mode uses `gpt-5.3-codex`.

@spec PROJECTOR.QUALITY.RELEASE_REVIEW.FAST_MODEL
the fast release-review mode uses `gpt-5.3-codex-spark`.

@spec PROJECTOR.QUALITY.RELEASE_REVIEW.SMART_MODEL
the smart release-review mode uses `gpt-5.4`.

@spec PROJECTOR.QUALITY.RELEASE_REVIEW.STRUCTURED_OUTPUT
the release-review wrapper validates structured warning output against the review contract.

@spec PROJECTOR.QUALITY.RELEASE_REVIEW.CODE_ONLY_SURFACE
release review operates only on the repo's code and tooling surface, not product/spec/architecture prose.

@spec PROJECTOR.QUALITY.RELEASE_REVIEW.READ_ONLY_SANDBOX
release review invokes Codex in a read-only sandbox.

@spec PROJECTOR.QUALITY.RELEASE_REVIEW.NO_WEB
release review disables web access.

@spec PROJECTOR.QUALITY.RELEASE_REVIEW.PROJECT_ROOT_READ_SCOPE
release review grants read access only to the project root.

@spec PROJECTOR.QUALITY.RELEASE_REVIEW.DIFF_SCOPED_BY_DEFAULT
without `--full`, release review is diff-scoped against the baseline tag.

@spec PROJECTOR.QUALITY.RELEASE_REVIEW.JJ_LATEST_TAG_BASELINE
release review uses the latest reachable semver tag as the default baseline.

@spec PROJECTOR.QUALITY.RELEASE_REVIEW.SYNTAX_AWARE_CHANGED_CONTEXT
release review extracts syntax-aware changed context for supported languages.

@spec PROJECTOR.QUALITY.RELEASE_REVIEW.INPUT_BUDGET
release review budgets prompt input before invoking Codex.

@spec PROJECTOR.QUALITY.RELEASE_REVIEW.CHUNKED_CONTEXT
release review splits review context into chunks when needed to fit the input budget.

@spec PROJECTOR.QUALITY.RELEASE_REVIEW.SKIPPED_CHUNK_WARNINGS
release review emits runner warnings when it must skip or degrade chunk context.

@spec PROJECTOR.QUALITY.RELEASE_REVIEW.FULL_SCAN_MODE
`--full` makes release review operate on the full supported review surface instead of the diff-scoped default.

@spec PROJECTOR.QUALITY.RELEASE_REVIEW.WARN_ONLY
release review reports findings as warnings rather than failing the wrapper on model findings alone.

@spec PROJECTOR.QUALITY.RELEASE_REVIEW.LOCAL_ONLY
release review runs locally and does not publish findings to external services.

@spec PROJECTOR.QUALITY.RELEASE_REVIEW.NO_BYTECODE_ARTIFACTS
release review does not leave Python bytecode artifacts in the repo.

@spec PROJECTOR.QUALITY.RELEASE_REVIEW.MANUAL_ONLY
release review runs only when invoked manually.

@module PROJECTOR.TESTS.QUALITY_RELEASE_REVIEW
Release-review wrapper tests in `tests/quality_release_review.rs`.
*/
// @fileimplements PROJECTOR.TESTS.QUALITY_RELEASE_REVIEW
#[path = "support/quality.rs"]
mod support;

use serde_json::json;
use std::{fs, process::Command};

use support::{
    latest_reachable_semver_tag, python_entrypoint_runtime_flag, release_review_changed_line_ranges,
    release_review_chunk_helper, release_review_dry_run, release_review_extract_context_ranges,
    release_review_extract_full_scan_context_ranges, release_review_merge_responses,
    release_review_passes_for, release_review_schema, release_review_validate_response_shape_err,
    release_review_validate_response_shape_ok, repo_root, workflow_files,
};

#[test]
// @verifies PROJECTOR.QUALITY.RELEASE_REVIEW.DEFAULT_MODEL
fn release_review_defaults_to_regular_53_model() {
    let payload = release_review_dry_run(&[]);
    assert_eq!(payload["model"], "gpt-5.3-codex");
    assert_eq!(payload["review_mode"], "default");
}

#[test]
// @verifies PROJECTOR.QUALITY.RELEASE_REVIEW.FAST_MODEL
fn release_review_uses_fast_model_when_requested() {
    let payload = release_review_dry_run(&["--fast"]);
    assert_eq!(payload["model"], "gpt-5.3-codex-spark");
    assert_eq!(payload["review_mode"], "fast");
}

#[test]
// @verifies PROJECTOR.QUALITY.RELEASE_REVIEW.SMART_MODEL
fn release_review_uses_smart_model_when_requested() {
    let payload = release_review_dry_run(&["--smart"]);
    assert_eq!(payload["model"], "gpt-5.4");
    assert_eq!(payload["review_mode"], "smart");
}

#[test]
// @verifies PROJECTOR.QUALITY.RELEASE_REVIEW.STRUCTURED_OUTPUT
fn release_review_validator_accepts_valid_payload_shape() {
    let validated = release_review_validate_response_shape_ok(
        r#"{
          "baseline": "v0.2.0",
          "full_scan": false,
          "summary": "clean",
          "warnings": []
        }"#,
    );
    assert_eq!(validated["summary"], "clean");

    let schema = release_review_schema();
    assert_eq!(schema["type"], "object");
    assert!(schema["properties"]["warnings"].is_object());
}

#[test]
// @verifies PROJECTOR.QUALITY.RELEASE_REVIEW.STRUCTURED_OUTPUT
fn release_review_validator_rejects_schema_drift() {
    let stderr = release_review_validate_response_shape_err(
        r#"{
          "baseline": null,
          "full_scan": true,
          "summary": "clean",
          "warnings": [],
          "unexpected": true
        }"#,
    );
    assert!(stderr.contains("unexpected keys"), "stderr:\n{}", stderr);
}

#[test]
// @verifies PROJECTOR.QUALITY.RELEASE_REVIEW.READ_ONLY_SANDBOX
fn release_review_script_uses_read_only_sandbox() {
    let payload = release_review_dry_run(&[]);
    assert_eq!(payload["codex_invocation"]["sandbox_mode"], "read-only");
}

#[test]
// @verifies PROJECTOR.QUALITY.RELEASE_REVIEW.NO_WEB
fn release_review_script_disables_web_search() {
    let payload = release_review_dry_run(&[]);
    assert_eq!(payload["codex_invocation"]["web_search"], "disabled");
}

#[test]
// @verifies PROJECTOR.QUALITY.RELEASE_REVIEW.PROJECT_ROOT_READ_SCOPE
fn release_review_script_uses_explicit_project_root_read_scope() {
    let payload = release_review_dry_run(&[]);
    assert_eq!(
        payload["codex_invocation"],
        json!({
            "model": "gpt-5.3-codex",
            "sandbox_mode": "read-only",
            "web_search": "disabled",
            "default_permissions": "release_review",
            "filesystem_permissions": {
                ":project_roots": {
                    ".": "read"
                }
            }
        })
    );
}

#[test]
// @verifies PROJECTOR.QUALITY.RELEASE_REVIEW.CODE_ONLY_SURFACE
fn release_review_full_scan_stays_on_code_surface() {
    let payload = release_review_dry_run(&["--full"]);
    let changed_files = payload["changed_files"]
        .as_array()
        .expect("changed_files should be an array");

    assert!(
        changed_files
            .iter()
            .any(|value| value.as_str() == Some("scripts/tag-release.py"))
    );
    assert!(
        changed_files
            .iter()
            .all(|value| !value.as_str().expect("file path").starts_with("specs/"))
    );
    assert!(
        changed_files
            .iter()
            .all(|value| !matches!(value.as_str(), Some("README.md" | "PRODUCT.md" | "ARCHITECTURE.md")))
    );
}

#[test]
// @verifies PROJECTOR.QUALITY.RELEASE_REVIEW.DIFF_SCOPED_BY_DEFAULT
fn release_review_defaults_to_diff_scope() {
    let payload = release_review_dry_run(&[]);
    assert_eq!(payload["full_scan"], false);
    assert!(payload["baseline"].is_string());
}

#[test]
// @verifies PROJECTOR.QUALITY.RELEASE_REVIEW.JJ_LATEST_TAG_BASELINE
fn release_review_defaults_to_latest_reachable_semver_tag_in_jj_repo() {
    let payload = release_review_dry_run(&[]);
    assert_eq!(payload["backend"], "jj");
    assert_eq!(payload["baseline"], latest_reachable_semver_tag());
}

#[test]
// @verifies PROJECTOR.QUALITY.RELEASE_REVIEW.SYNTAX_AWARE_CHANGED_CONTEXT
fn release_review_extracts_local_rust_item_context() {
    let content = "\
use std::path::PathBuf;
\n\
#[derive(Clone)]\n\
struct Example {\n\
    name: String,\n\
}\n\
\n\
fn build_example() -> Example {\n\
    Example { name: \"demo\".to_owned() }\n\
}\n";

    let ranges = release_review_extract_context_ranges("src/example.rs", content, 7, 7);
    assert_eq!(ranges, json!([[3, 6]]));
}

#[test]
fn release_review_full_scan_covers_unmatched_rust_preamble() {
    let content = "\
use std::path::PathBuf;\n\
use std::time::Duration;\n\
\n\
fn build() {}\n";

    let ranges = release_review_extract_full_scan_context_ranges("src/example.rs", content);
    assert_eq!(ranges, json!([[1, 4]]));
}

#[test]
// @verifies PROJECTOR.QUALITY.RELEASE_REVIEW.INPUT_BUDGET
fn release_review_chunks_stay_within_budget() {
    let payload = release_review_dry_run(&["--full"]);
    for review_pass in payload["review_passes"]
        .as_array()
        .expect("review_passes should be an array")
    {
        for chunk in review_pass["chunks"]
            .as_array()
            .expect("chunks should be an array")
        {
            let estimated = chunk["estimated_chars"]
                .as_i64()
                .expect("estimated_chars should be an integer");
            assert!(
                estimated <= 128_000,
                "chunk should stay within the configured budget: {estimated}"
            );
        }
    }
}

#[test]
// @verifies PROJECTOR.QUALITY.RELEASE_REVIEW.CHUNKED_CONTEXT
fn release_review_splits_oversized_context_into_multiple_chunks() {
    let payload = release_review_chunk_helper(80_000);
    let chunks = payload["chunks"].as_array().expect("chunks should be an array");
    assert!(chunks.len() >= 2, "expected multiple chunks, got {}", chunks.len());
}

#[test]
// @verifies PROJECTOR.QUALITY.RELEASE_REVIEW.SKIPPED_CHUNK_WARNINGS
fn release_review_reports_runner_warning_for_unsendable_chunk() {
    let payload = release_review_chunk_helper(140_000);
    let warnings = payload["runner_warnings"]
        .as_array()
        .expect("runner_warnings should be an array");
    assert!(
        warnings.iter().any(|warning| warning
            .as_str()
            .expect("warning should be a string")
            .contains("skipped")),
        "runner warnings should describe skipped context: {warnings:?}"
    );
}

#[test]
fn release_review_adds_default_pass_for_unmatched_changed_files() {
    let passes = release_review_passes_for(&["src/unknown.rs"]);
    assert_eq!(passes[0]["name"], "default");
}

#[test]
fn release_review_merge_output_is_stable_across_response_order() {
    let responses = json!([
        [
            "server_runtime",
            1,
            {
                "baseline": "v0.2.0",
                "full_scan": false,
                "summary": "warn",
                "warnings": [{
                    "id": "chunk-one",
                    "category": "maintainability",
                    "severity": "warn",
                    "title": "Example warning",
                    "why_it_matters": "Stability matters.",
                    "evidence": [{"path": "scripts/review-projector-release-style.py", "line": 1, "detail": "anchor"}],
                    "recommendation": "Keep ids stable."
                }]
            }
        ]
    ]);

    let first = release_review_merge_responses(&responses);
    let reversed = json!(responses.as_array().expect("array").iter().rev().collect::<Vec<_>>());
    let second = release_review_merge_responses(&reversed);
    assert_eq!(first, second);
}

#[test]
// @verifies PROJECTOR.QUALITY.RELEASE_REVIEW.FULL_SCAN_MODE
fn release_review_supports_full_scan_mode() {
    let payload = release_review_dry_run(&["--full"]);
    assert_eq!(payload["full_scan"], true);
    assert!(payload["baseline"].is_null());
}

#[test]
fn release_review_tracks_actual_changed_lines_not_hunk_header_ranges() {
    let ranges = release_review_changed_line_ranges(
        "\
diff --git a/src/example.rs b/src/example.rs\n\
index 1111111..2222222 100644\n\
--- a/src/example.rs\n\
+++ b/src/example.rs\n\
@@ -8,3 +8,4 @@\n\
 line8\n\
-line9\n\
+line9 changed\n\
 line10\n\
+line11 added\n",
    );
    assert_eq!(ranges, json!({"src/example.rs": [[9, 9], [11, 11]]}));
}

#[test]
// @verifies PROJECTOR.QUALITY.RELEASE_REVIEW.WARN_ONLY
fn release_review_exits_successfully_when_codex_returns_warnings() {
    let output = Command::new("python3")
        .arg("scripts/review-projector-release-style.py")
        .arg("--full")
        .arg("--allow-mock")
        .current_dir(repo_root())
        .env("PROJECTOR_RELEASE_REVIEW_ALLOW_MOCK", "1")
        .env(
            "PROJECTOR_RELEASE_REVIEW_MOCK_OUTPUT",
            r#"{
              "baseline": "v0.2.0",
              "full_scan": false,
              "summary": "warn",
              "warnings": [{
                "id": "warn-1",
                "category": "maintainability",
                "severity": "warn",
                "title": "Example warning",
                "why_it_matters": "Warnings should stay warn-only.",
                "evidence": [{"path":"src/server/src/main.rs","line":1,"detail":"anchor"}],
                "recommendation": "Review before tagging."
              }]
            }"#,
        )
        .output()
        .expect("release review should run");

    assert!(
        output.status.success(),
        "stdout:\n{}\n\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
// @verifies PROJECTOR.QUALITY.RELEASE_REVIEW.LOCAL_ONLY
fn release_review_refuses_live_codex_invocation_in_ci() {
    let output = Command::new("python3")
        .arg("scripts/review-projector-release-style.py")
        .current_dir(repo_root())
        .env("CI", "1")
        .output()
        .expect("release review should run");

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("local-only"),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
// @verifies PROJECTOR.QUALITY.RELEASE_REVIEW.NO_BYTECODE_ARTIFACTS
fn release_review_entrypoint_sets_no_bytecode_runtime_flag() {
    let review_flag = python_entrypoint_runtime_flag("review-projector-release-style.py");
    let tag_flag = python_entrypoint_runtime_flag("tag-release.py");

    assert_eq!(review_flag["dont_write_bytecode"], true);
    assert_eq!(tag_flag["dont_write_bytecode"], true);
}

#[test]
// @verifies PROJECTOR.QUALITY.RELEASE_REVIEW.MANUAL_ONLY
fn release_review_is_not_wired_into_ci_workflows_or_release_publication() {
    for workflow in workflow_files() {
        let contents =
            fs::read_to_string(&workflow).expect("workflow file should be readable as utf-8");
        assert!(
            !contents.contains("review-projector-release-style.py"),
            "workflow {} should not invoke the local codex release review directly",
            workflow.display()
        );
    }

    let tag_release = fs::read_to_string(repo_root().join("scripts/tag-release.py"))
        .expect("tag-release.py should be readable");
    assert!(
        !tag_release.contains("review-projector-release-style.py"),
        "release publication should not invoke the local codex review script"
    );
}
