#!/usr/bin/env python3
# Local release publication flow in `scripts/tag-release.py`.

from __future__ import annotations

import sys

sys.dont_write_bytecode = True

import argparse
import json
import os
import subprocess
import time
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
if str(SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPT_DIR))

from release_tooling import normalize_tag, package_version, run_checked

CHECKLIST = [
    {
        "id": "readme",
        "prompt": "Updated README.md and other public docs for this release?",
    },
    {
        "id": "changelog",
        "prompt": "Updated CHANGELOG.md for this release?",
    },
    {
        "id": "version",
        "prompt": "Bumped workspace package versions and release references to this version?",
    },
    {
        "id": "validation",
        "prompt": "Ran core validation (`cargo test`, `special lint`, `special specs`)?",
    },
    {
        "id": "release_review",
        "prompt": "Ran the local release review and looked at any warn-level findings?",
    },
]
GITHUB_RELEASE_POLL_SECONDS = 10
GITHUB_RELEASE_TIMEOUT_SECONDS = 15 * 60


def repo_root() -> Path:
    return Path(__file__).resolve().parent.parent


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Publish the current revision by updating main, tagging the release, pushing both, "
            "verifying the GitHub release, and updating Homebrew."
        )
    )
    parser.add_argument("version", help="Release version, with or without a leading `v`.")
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Print the planned checklist and publication commands without publishing.",
    )
    parser.add_argument(
        "--skip-checklist",
        dest="skip_checklist",
        action="store_true",
        help="Bypass the interactive prerelease checklist.",
    )
    parser.add_argument("--yes", dest="skip_checklist", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--allow-mock-publish", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--allow-existing-tag", action="store_true", help=argparse.SUPPRESS)
    return parser.parse_args()


def existing_tags(root: Path) -> set[str]:
    output = run_checked(root, ["jj", "--ignore-working-copy", "tag", "list"])
    return {line.split(":", 1)[0].strip() for line in output.splitlines() if ":" in line}


def current_revision(root: Path) -> str:
    return run_checked(
        root,
        ["jj", "--ignore-working-copy", "log", "-r", "@-", "--no-graph", "-T", "commit_id"],
    ).strip()


def tag_revision(root: Path, tag: str) -> str:
    return run_checked(
        root,
        ["jj", "--ignore-working-copy", "log", "-r", tag, "--no-graph", "-T", "commit_id"],
    ).strip()


def prompt_checklist() -> None:
    print("Prerelease checklist:")
    for item in CHECKLIST:
        try:
            answer = input(f" - {item['prompt']} [y/N]: ").strip().lower()
        except EOFError as err:
            raise SystemExit(
                "interactive release checklist is unavailable; rerun with --skip-checklist to publish"
            ) from err
        if answer not in {"y", "yes"}:
            raise SystemExit("aborted release publishing")


def run_command(root: Path, command: list[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        command,
        cwd=root,
        capture_output=True,
        text=True,
    )


def record_mock_step(root: Path, label: str, command: list[str]) -> None:
    log_override = os.environ.get("PROJECTOR_RELEASE_MOCK_LOG_PATH")
    log_path = Path(log_override) if log_override else root / ".tmp-release-mock-log"
    with log_path.open("a", encoding="utf-8") as handle:
        handle.write(json.dumps({"label": label, "command": command}) + "\n")


def run_step(root: Path, label: str, command: list[str], args: argparse.Namespace) -> None:
    if args.allow_mock_publish:
        record_mock_step(root, label, command)
        return
    run_checked(root, command)


def wait_for_github_release(root: Path, label: str, command: list[str], args: argparse.Namespace) -> None:
    if args.allow_mock_publish:
        record_mock_step(root, label, command)
        return
    deadline = time.monotonic() + GITHUB_RELEASE_TIMEOUT_SECONDS
    last_stderr = ""
    while True:
        result = run_command(root, command)
        if result.returncode == 0:
            return
        last_stderr = result.stderr
        if time.monotonic() >= deadline:
            if last_stderr:
                sys.stderr.write(last_stderr)
            raise SystemExit("timed out waiting for GitHub release publication")
        time.sleep(GITHUB_RELEASE_POLL_SECONDS)


def main() -> int:
    args = parse_args()
    root = repo_root()
    tag = normalize_tag(args.version)
    manifest_tag = normalize_tag(package_version(root))

    if not root.joinpath(".jj").exists():
        raise SystemExit("release publishing requires a jj repository root")
    revision = current_revision(root)
    tag_exists = tag in existing_tags(root)
    if tag_exists and not args.allow_existing_tag:
        raise SystemExit(f"release tag `{tag}` already exists")
    if tag != manifest_tag:
        raise SystemExit(
            f"release tag `{tag}` does not match workspace version `{manifest_tag}`"
        )

    bookmark_command = ["jj", "bookmark", "set", "main", "-r", revision]
    tag_command = ["jj", "tag", "set", tag, "-r", revision]
    if tag_exists and args.allow_existing_tag:
        tag_command.insert(3, "--allow-move")
    push_main_command = ["jj", "git", "push", "--bookmark", "main"]
    push_tag_command = ["git", "push"]
    if tag_exists and args.allow_existing_tag:
        push_tag_command.append("--force")
    push_tag_command.extend(["origin", f"refs/tags/{tag}"])
    verify_github_release_command = [
        "bash",
        str(SCRIPT_DIR / "verify-github-release-published.sh"),
    ]
    update_homebrew_formula_command = [
        sys.executable,
        str(SCRIPT_DIR / "update-homebrew-formula.py"),
    ]
    verify_homebrew_formula_command = [
        "bash",
        str(SCRIPT_DIR / "verify-homebrew-formula.sh"),
    ]

    if args.dry_run:
        print(
            json.dumps(
                {
                    "tag": tag,
                    "revision": revision,
                    "checklist": CHECKLIST,
                    "bookmark_command": bookmark_command,
                    "tag_command": tag_command,
                    "push_main_command": push_main_command,
                    "push_tag_command": push_tag_command,
                    "verify_github_release_command": verify_github_release_command,
                    "update_homebrew_formula_command": update_homebrew_formula_command,
                    "verify_homebrew_formula_command": verify_homebrew_formula_command,
                },
                indent=2,
            )
        )
        return 0

    if not args.skip_checklist:
        prompt_checklist()

    run_step(root, "bookmark_main", bookmark_command, args)
    if not (tag_exists and tag_revision(root, tag) == revision):
        run_step(root, "set_tag", tag_command, args)
    run_step(root, "push_main", push_main_command, args)
    run_step(root, "push_tag", push_tag_command, args)
    wait_for_github_release(root, "verify_github_release", verify_github_release_command, args)
    run_step(root, "update_homebrew_formula", update_homebrew_formula_command, args)
    run_step(root, "verify_homebrew_formula", verify_homebrew_formula_command, args)
    print(f"Published release {tag}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
