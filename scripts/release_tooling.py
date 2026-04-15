#!/usr/bin/env python3
# Shared release helpers for projector publication scripts.

from __future__ import annotations

import re
import subprocess
import sys
import tomllib
from pathlib import Path

SEMVER_RE = re.compile(r"^v?(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$")


def normalize_tag(value: str, *, subject: str = "release version") -> str:
    match = SEMVER_RE.match(value)
    if not match:
        raise SystemExit(
            f"{subject} must be semver like `0.1.0` or `v0.1.0`, got `{value}`"
        )
    return f"v{match.group(1)}.{match.group(2)}.{match.group(3)}" + (
        f"-{match.group(4)}" if match.group(4) else ""
    )


def workspace_package_versions(root: Path) -> dict[str, str]:
    workspace = tomllib.loads(root.joinpath("Cargo.toml").read_text())
    members = workspace["workspace"]["members"]
    versions: dict[str, str] = {}
    for member in members:
        manifest_path = root.joinpath(member, "Cargo.toml")
        manifest = tomllib.loads(manifest_path.read_text())
        package = manifest.get("package")
        if package is None:
            continue
        versions[str(package["name"])] = str(package["version"])
    return versions


def package_version(root: Path) -> str:
    versions = workspace_package_versions(root)
    unique_versions = sorted(set(versions.values()))
    if len(unique_versions) != 1:
        raise SystemExit(f"workspace packages do not share one version: {versions}")
    return unique_versions[0]


def run_checked(root: Path, command: list[str]) -> str:
    result = subprocess.run(
        command,
        cwd=root,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        sys.stderr.write(result.stderr)
        raise SystemExit(result.returncode)
    return result.stdout
