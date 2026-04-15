#!/usr/bin/env python3
# Homebrew formula update automation for projector.

from __future__ import annotations

import sys

sys.dont_write_bytecode = True

import base64
import json
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
if str(SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPT_DIR))

from release_tooling import normalize_tag, package_version, run_checked

REPOSITORY = "LabLeaks/projector"
TAP_REPOSITORY = "LabLeaks/homebrew-tap"
FORMULA_PATH = "Formula/projector.rb"
FORMULA_DESC = "Private context sync and restore CLI"
FORMULA_HOMEPAGE = "https://github.com/LabLeaks/projector"


def repo_root() -> Path:
    return Path(__file__).resolve().parent.parent


def release_assets(root: Path, tag: str) -> dict[str, dict]:
    payload = json.loads(
        run_checked(
            root,
            ["gh", "release", "view", tag, "--repo", REPOSITORY, "--json", "assets"],
        )
    )
    return {asset["name"]: asset for asset in payload["assets"]}


def formula_file(root: Path) -> tuple[str, str]:
    payload = json.loads(
        run_checked(
            root,
            ["gh", "api", f"repos/{TAP_REPOSITORY}/contents/{FORMULA_PATH}"],
        )
    )
    content = base64.b64decode(payload["content"]).decode()
    return content, str(payload["sha"])


def release_asset_names(root: Path) -> list[str]:
    payload = json.loads(root.joinpath("scripts/release-assets.json").read_text())
    return list(payload["homebrew_formula_archives"])


def asset_sha256(asset: dict) -> str:
    digest = str(asset["digest"])
    if not digest.startswith("sha256:"):
        raise SystemExit(f"release asset digest is not sha256: {asset}")
    return digest.removeprefix("sha256:")


def build_formula(version: str, assets: dict[str, dict]) -> str:
    def archive_sha(name: str) -> str:
        return asset_sha256(assets[name])

    return f"""# typed: false
# frozen_string_literal: true

class Projector < Formula
  desc "{FORMULA_DESC}"
  homepage "{FORMULA_HOMEPAGE}"
  version "{version}"

  archive = on_system_conditional(
    macos: on_arch_conditional(
      arm: "projector-cli-aarch64-apple-darwin.tar.xz",
      intel: "projector-cli-x86_64-apple-darwin.tar.xz",
    ),
    linux: on_arch_conditional(
      arm: "projector-cli-aarch64-unknown-linux-gnu.tar.xz",
      intel: "projector-cli-x86_64-unknown-linux-gnu.tar.xz",
    ),
  )
  url "https://github.com/LabLeaks/projector/releases/download/v{version}/#{{archive}}"
  sha256 on_system_conditional(
    macos: on_arch_conditional(
      arm: "{archive_sha('projector-cli-aarch64-apple-darwin.tar.xz')}",
      intel: "{archive_sha('projector-cli-x86_64-apple-darwin.tar.xz')}",
    ),
    linux: on_arch_conditional(
      arm: "{archive_sha('projector-cli-aarch64-unknown-linux-gnu.tar.xz')}",
      intel: "{archive_sha('projector-cli-x86_64-unknown-linux-gnu.tar.xz')}",
    ),
  )

  def install
    bin.install "projector"
  end

  test do
    system "#{{bin}}/projector", "--help"
  end
end
"""


def main() -> int:
    root = repo_root()
    version = package_version(root)
    tag = normalize_tag(version)
    assets = release_assets(root, tag)
    required = release_asset_names(root)
    missing = sorted(set(required) - assets.keys())
    if missing:
        raise SystemExit(f"missing required release assets for Homebrew formula: {missing}")

    _, sha = formula_file(root)
    formula = build_formula(version, assets)
    encoded = base64.b64encode(formula.encode()).decode()
    run_checked(
        root,
        [
            "gh",
            "api",
            f"repos/{TAP_REPOSITORY}/contents/{FORMULA_PATH}",
            "--method",
            "PUT",
            "-f",
            f"message=Update Formula/projector.rb for {tag}",
            "-f",
            f"content={encoded}",
            "-f",
            f"sha={sha}",
            "-f",
            "branch=main",
        ],
    )
    print(f"Updated {TAP_REPOSITORY}/{FORMULA_PATH} for {tag}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
