#!/usr/bin/env bash
# Homebrew formula verification for projector.

set -euo pipefail

cd "$(dirname "$0")/.."

version="$(python3 - <<'PY'
from pathlib import Path
import sys

sys.path.insert(0, str(Path("scripts").resolve()))
from release_tooling import package_version

print(package_version(Path(".")))
PY
)"

release_json="$(gh release view "v${version}" --repo LabLeaks/projector --json assets)"
formula="$(gh api repos/LabLeaks/homebrew-tap/contents/Formula/projector.rb --jq .content | base64 --decode)"

FORMULA_TEXT="$formula" python3 - "$version" "$release_json" <<'PY'
import json
import os
import re
import sys
from pathlib import Path

version = sys.argv[1]
release = json.loads(sys.argv[2])
formula = os.environ["FORMULA_TEXT"]

assets = {asset["name"]: asset for asset in release["assets"]}
required = set(
    json.loads(Path("scripts/release-assets.json").read_text())["homebrew_formula_archives"]
)
missing = sorted(required - assets.keys())
selector_arms = {
    "projector-cli-aarch64-apple-darwin.tar.xz": ("macos", "arm"),
    "projector-cli-x86_64-apple-darwin.tar.xz": ("macos", "intel"),
    "projector-cli-aarch64-unknown-linux-gnu.tar.xz": ("linux", "arm"),
    "projector-cli-x86_64-unknown-linux-gnu.tar.xz": ("linux", "intel"),
}

def fail(message, details):
    raise SystemExit(f"{message}: {details}")

def asset_sha256(asset):
    digest = asset.get("digest")
    if digest is None:
        fail("release asset is missing digest", asset)
    if not isinstance(digest, str) or not digest.startswith("sha256:"):
        fail("release asset digest is not sha256", asset)
    sha256 = digest.removeprefix("sha256:")
    if not re.fullmatch(r"[0-9a-f]{64}", sha256):
        fail("release asset digest is not a valid sha256", asset)
    return sha256

if missing:
    fail("missing required release assets for Homebrew formula", missing)
if "class Projector < Formula" not in formula:
    fail("formula class declaration missing", "class Projector < Formula")
if f'version "{version}"' not in formula:
    fail("formula version mismatch", version)
if 'bin.install "projector"' not in formula:
    fail("formula no longer installs projector", formula)
for helper in ('on_system_conditional(', 'on_arch_conditional('):
    if helper not in formula:
        fail("formula is missing platform selection helper", helper)
expected_url = f'https://github.com/LabLeaks/projector/releases/download/v{version}/#{{archive}}'
if f'url "{expected_url}"' not in formula:
    fail("formula is missing templated release asset url", expected_url)

for name in sorted(required):
    asset = assets[name]
    sha256 = asset_sha256(asset)
    selector = selector_arms.get(name)
    if selector is None:
        fail("required release asset has no Homebrew selector mapping", name)
    os_name, arch = selector
    archive_pattern = re.compile(
        rf'archive\s*=\s*on_system_conditional\(\s*.*?{os_name}\s*:\s*on_arch_conditional\(\s*.*?{arch}\s*:\s*"{re.escape(name)}"',
        re.S,
    )
    if not archive_pattern.search(formula):
        fail("formula is missing archive selector entry", name)
    checksum_pattern = re.compile(
        rf'{os_name}\s*:\s*on_arch_conditional\(\s*.*?{arch}\s*:\s*"{re.escape(sha256)}"',
        re.S,
    )
    if not checksum_pattern.search(formula):
        fail(
            "formula checksum selector entry does not contain expected checksum",
            {"archive": name, "os": os_name, "arch": arch, "sha256": sha256},
        )
PY
