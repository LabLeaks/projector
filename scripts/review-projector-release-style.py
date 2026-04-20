#!/usr/bin/env python3
# @module PROJECTOR.QUALITY.RELEASE_REVIEW
# Local-only code-quality review entrypoint that orchestrates release review and surfaces release warnings for tagging. Context planning, payload validation, Codex invocation policy, and response merging live in dedicated helper modules.
# @fileimplements PROJECTOR.QUALITY.RELEASE_REVIEW
# @fileverifies PROJECTOR.QUALITY.RELEASE_REVIEW.SPEC_OWNED

from __future__ import annotations

import sys

sys.dont_write_bytecode = True

import argparse
import concurrent.futures
import json
import os
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
if str(SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPT_DIR))

from release_review_contract import validate_review_payload, validate_review_preview
from release_review_invoke import (
    DEFAULT_MODEL,
    FAST_MODEL,
    MOCK_ALLOW_ENV,
    SMART_MODEL,
    CodexInvocationError,
    codex_invocation_config,
    invoke_codex,
)
from release_review_merge import merge_pass_responses
from release_review_pipeline import (
    MAX_CONCURRENT_REVIEW_CHUNKS,
    build_file_contexts,
    build_pass_chunks,
    build_review_passes,
    changed_files_from_diff,
    command_exists,
    discover_latest_semver_tag,
    diff_text_for_paths,
    extract_context_ranges,
    full_scan_files,
    parse_changed_line_ranges,
)
from release_tooling import package_version


SCHEMA_PATH = Path(__file__).with_name("projector-release-review.schema.json")


def repo_root() -> Path:
    return Path(__file__).resolve().parent.parent


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Run a warn-only Codex release review for projector implementation-quality issues."
        )
    )
    parser.add_argument(
        "--base",
        help="Explicit base tag or revision to diff against. Defaults to the latest semver tag.",
    )
    parser.add_argument(
        "--full",
        action="store_true",
        help="Review the full projector code/tooling surface instead of only the release diff.",
    )
    parser.add_argument("--head", help=argparse.SUPPRESS)
    model_group = parser.add_mutually_exclusive_group()
    model_group.add_argument(
        "--fast",
        action="store_true",
        help=f"Use the faster {FAST_MODEL} review model.",
    )
    model_group.add_argument(
        "--smart",
        action="store_true",
        help=f"Use the stronger {SMART_MODEL} review model.",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Print the assembled review payload as JSON instead of invoking Codex.",
    )
    parser.add_argument("--allow-mock", action="store_true", help=argparse.SUPPRESS)
    return parser.parse_args()


def running_in_ci() -> bool:
    return any(
        os.environ.get(name, "").strip()
        for name in ("CI", "GITHUB_ACTIONS", "BUILDKITE", "JENKINS_URL")
    )


def selected_model(args: argparse.Namespace) -> tuple[str, str]:
    if args.fast:
        return ("fast", FAST_MODEL)
    if args.smart:
        return ("smart", SMART_MODEL)
    return ("default", DEFAULT_MODEL)


def has_jj_root(root: Path) -> bool:
    return root.joinpath(".jj").exists() and command_exists("jj")


def has_git_root(root: Path) -> bool:
    return root.joinpath(".git").exists() and command_exists("git")


def load_version(root: Path) -> str:
    return package_version(root)


def validate_response_shape(response: dict) -> dict:
    return validate_review_payload(response, subject="review response")


def main() -> int:
    args = parse_args()
    root = repo_root()

    if not args.allow_mock:
        forbidden_mock_envs = [
            name
            for name in (
                MOCK_ALLOW_ENV,
                "PROJECTOR_RELEASE_REVIEW_MOCK_OUTPUT",
                "PROJECTOR_RELEASE_REVIEW_MOCK_EXIT_CODE",
            )
            if os.environ.get(name) is not None
        ]
        if forbidden_mock_envs:
            raise SystemExit(
                "release review mock controls are test-only; unset "
                + ", ".join(sorted(forbidden_mock_envs))
            )

    if has_jj_root(root):
        backend = "jj"
    elif has_git_root(root):
        backend = "git"
    else:
        raise SystemExit("repository root must contain .jj or .git")

    review_mode, model = selected_model(args)
    version = load_version(root)
    head = args.head or ("@-" if backend == "jj" else "HEAD")
    base = None if args.full else (args.base or discover_latest_semver_tag(root, backend, head))
    review_files = (
        full_scan_files(root, backend)
        if args.full
        else changed_files_from_diff(root, backend, base, head)
    )
    review_passes = build_review_passes(review_files)

    changed_line_ranges: dict[str, list[tuple[int, int]]] = {}
    if not args.full and review_files and base is not None:
        changed_line_ranges = parse_changed_line_ranges(
            diff_text_for_paths(root, backend, base, head, review_files)
        )

    chunk_records: list[dict[str, object]] = []
    runner_warnings: list[str] = []

    for review_pass in review_passes:
        pass_files = list(review_pass["files"])
        file_contexts = build_file_contexts(root, pass_files, changed_line_ranges, args.full)
        chunks, pass_runner_warnings = build_pass_chunks(
            root,
            version,
            backend,
            base,
            head,
            args.full,
            review_pass,
            file_contexts,
        )
        runner_warnings.extend(pass_runner_warnings)
        chunk_records.extend(chunks)

    if args.dry_run:
        preview = validate_review_preview(
            {
                "model": model,
                "review_mode": review_mode,
                "codex_invocation": codex_invocation_config(model),
                "schema_path": str(SCHEMA_PATH),
                "backend": backend,
                "baseline": base,
                "head": head,
                "full_scan": args.full,
                "changed_files": review_files,
                "runner_warnings": runner_warnings,
                "review_passes": [
                    {
                        "name": review_pass["name"],
                        "focus": review_pass["focus"],
                        "files": review_pass["files"],
                        "chunks": [
                            {
                                "chunk_index": chunk["chunk_index"],
                                "chunk_count": chunk["chunk_count"],
                                "files": chunk["files"],
                                "estimated_chars": chunk["estimated_chars"],
                                "file_contexts": chunk["file_contexts"],
                                "prompt": chunk["prompt"],
                            }
                            for chunk in chunk_records
                            if chunk["name"] == review_pass["name"]
                        ],
                    }
                    for review_pass in review_passes
                ],
            },
            subject="review dry-run preview",
        )
        print(json.dumps(preview, indent=2))
        return 0

    if running_in_ci():
        raise SystemExit(
            "projector release review is local-only and must not invoke Codex from CI; "
            "use --dry-run for wrapper verification"
        )

    if not chunk_records:
        payload = merge_pass_responses(base, args.full, [], runner_warnings)
        for warning in runner_warnings:
            print(warning, file=sys.stderr)
        print(json.dumps(payload, indent=2))
        return 1 if runner_warnings else 0

    responses: list[tuple[str, int, dict]] = []
    max_workers = min(len(chunk_records), MAX_CONCURRENT_REVIEW_CHUNKS)

    with concurrent.futures.ThreadPoolExecutor(max_workers=max_workers or 1) as executor:
        future_to_chunk = {
            executor.submit(
                invoke_codex,
                root,
                str(chunk["prompt"]),
                model,
                review_mode,
                SCHEMA_PATH,
                validate_response_shape,
            ): chunk
            for chunk in chunk_records
        }
        for future in concurrent.futures.as_completed(future_to_chunk):
            chunk = future_to_chunk[future]
            try:
                response = future.result()
            except CodexInvocationError as error:
                runner_warnings.append(
                    f"review chunk {chunk['name']}#{chunk['chunk_index']} failed: {error}"
                )
                continue
            except BaseException as error:
                runner_warnings.append(
                    f"review chunk {chunk['name']}#{chunk['chunk_index']} crashed: {error}"
                )
                continue
            responses.append((str(chunk["name"]), int(chunk["chunk_index"]), response))

    responses.sort(key=lambda item: (item[0], item[1]))
    runner_warnings.sort()
    payload = merge_pass_responses(base, args.full, responses, runner_warnings)
    for warning in runner_warnings:
        print(warning, file=sys.stderr)
    print(json.dumps(payload, indent=2))
    mock_exit_code = os.environ.get("PROJECTOR_RELEASE_REVIEW_MOCK_EXIT_CODE")
    if mock_exit_code is not None and os.environ.get(MOCK_ALLOW_ENV) == "1":
        return int(mock_exit_code)
    return 1 if runner_warnings else 0


if __name__ == "__main__":
    raise SystemExit(main())
