#!/usr/bin/env bash
# GitHub Release publication verification for projector.

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

release_json="$(gh release view "v${version}" --repo LabLeaks/projector --json tagName,isDraft,isPrerelease,assets)"

python3 - "$version" "$release_json" <<'PY'
import json
import sys
from pathlib import Path

version = sys.argv[1]
release = json.loads(sys.argv[2])
expected_assets = set(
    json.loads(Path("scripts/release-assets.json").read_text())["github_release_assets"]
)
asset_names = {asset["name"] for asset in release["assets"]}
expected_prerelease = "-" in version

def fail(message, details):
    raise SystemExit(f"{message}: {details}")

if release["tagName"] != f"v{version}":
    fail("release tag mismatch", release)
if release["isDraft"] is not False:
    fail("release is still a draft", release)
if release["isPrerelease"] is not expected_prerelease:
    fail("release prerelease state mismatch", release)
if asset_names != expected_assets:
    fail(
        "release assets did not match expected set",
        {
            "missing": sorted(expected_assets - asset_names),
            "unexpected": sorted(asset_names - expected_assets),
        },
    )
PY
