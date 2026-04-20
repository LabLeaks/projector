# @module PROJECTOR.QUALITY.RELEASE_REVIEW.INVOKE
# Codex invocation policy and runtime error handling for local release review. This module does not choose review passes or merge warning payloads.
# @fileimplements PROJECTOR.QUALITY.RELEASE_REVIEW.INVOKE
from __future__ import annotations

import json
import os
import subprocess
from pathlib import Path


DEFAULT_MODEL = "gpt-5.3-codex"
FAST_MODEL = "gpt-5.3-codex-spark"
SMART_MODEL = "gpt-5.4"
PERMISSIONS_PROFILE = "release_review"
MOCK_ALLOW_ENV = "PROJECTOR_RELEASE_REVIEW_ALLOW_MOCK"
MOCK_OUTPUT_ENV = "PROJECTOR_RELEASE_REVIEW_MOCK_OUTPUT"


class CodexInvocationError(RuntimeError):
    pass


def quota_guidance(model_mode: str) -> str | None:
    if model_mode == "fast":
        return (
            f"{FAST_MODEL} appears quota-limited. Rerun without --fast for the default "
            f"{DEFAULT_MODEL} review, or use --smart for {SMART_MODEL}."
        )
    if model_mode == "smart":
        return (
            f"{SMART_MODEL} appears quota-limited. Rerun without --smart for the default "
            f"{DEFAULT_MODEL} review, or use --fast if Spark quota is available."
        )
    return None


def codex_invocation_config(model: str) -> dict[str, object]:
    return {
        "model": model,
        "sandbox_mode": "read-only",
        "web_search": "disabled",
        "default_permissions": PERMISSIONS_PROFILE,
        "filesystem_permissions": {
            ":project_roots": {
                ".": "read",
            }
        },
    }


def codex_exec_command(model: str, schema_path: Path) -> list[str]:
    config = codex_invocation_config(model)
    filesystem_toml = '{":project_roots"={"."="read"}}'
    return [
        "codex",
        "exec",
        "--ephemeral",
        "--sandbox",
        str(config["sandbox_mode"]),
        "-c",
        f'web_search="{config["web_search"]}"',
        "-c",
        f'default_permissions="{config["default_permissions"]}"',
        "-c",
        f"permissions.{config['default_permissions']}.filesystem={filesystem_toml}",
        "--skip-git-repo-check",
        "--model",
        model,
        "--output-schema",
        str(schema_path),
        "-",
    ]


def invoke_codex(
    root: Path,
    prompt: str,
    model: str,
    model_mode: str,
    schema_path: Path,
    validate_response_shape,
) -> dict:
    mocked = os.environ.get(MOCK_OUTPUT_ENV)
    if mocked and os.environ.get(MOCK_ALLOW_ENV) == "1":
        try:
            return validate_response_shape(json.loads(mocked))
        except (json.JSONDecodeError, SystemExit) as err:
            raise CodexInvocationError(f"mocked review output was invalid: {err}") from err

    result = subprocess.run(
        codex_exec_command(model, schema_path),
        cwd=root,
        input=prompt,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        stderr = result.stderr.strip()
        lower = stderr.lower()
        guidance = (
            quota_guidance(model_mode)
            if any(token in lower for token in ("quota", "rate limit", "usage limit", "5hr", "7day"))
            else None
        )
        if guidance:
            raise CodexInvocationError(f"{stderr}\n{guidance}")
        raise CodexInvocationError(stderr or f"codex exited with status {result.returncode}")

    try:
        return validate_response_shape(json.loads(result.stdout))
    except (json.JSONDecodeError, SystemExit) as err:
        raise CodexInvocationError(f"codex returned invalid structured output: {err}") from err
