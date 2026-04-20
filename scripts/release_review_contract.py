# @module PROJECTOR.QUALITY.RELEASE_REVIEW.CONTRACT
# Structured payload validation in `scripts/release_review_contract.py`.
# @fileimplements PROJECTOR.QUALITY.RELEASE_REVIEW.CONTRACT
from __future__ import annotations


ALLOWED_TOP_LEVEL_KEYS = {
    "baseline",
    "full_scan",
    "summary",
    "warnings",
}
ALLOWED_PREVIEW_TOP_LEVEL_KEYS = {
    "model",
    "review_mode",
    "codex_invocation",
    "schema_path",
    "backend",
    "baseline",
    "head",
    "full_scan",
    "changed_files",
    "runner_warnings",
    "review_passes",
}
ALLOWED_PREVIEW_PASS_KEYS = {"name", "focus", "files", "chunks"}
REQUIRED_PREVIEW_CHUNK_KEYS = {
    "chunk_index",
    "chunk_count",
    "files",
    "diff",
    "estimated_chars",
    "file_contexts",
    "prompt",
}
REQUIRED_FILE_CONTEXT_KEYS = {"path", "start_line", "end_line", "content"}
REQUIRED_WARNING_KEYS = {
    "id",
    "category",
    "severity",
    "title",
    "why_it_matters",
    "evidence",
    "recommendation",
}
ALLOWED_WARNING_CATEGORIES = {
    "api-design",
    "type-design",
    "state-model",
    "error-handling",
    "layering",
    "test-quality",
    "maintainability",
    "rust-idioms",
    "release-risk",
}


def _require_int(
    value: object,
    field_name: str,
    *,
    positive: bool = False,
    non_negative: bool = False,
) -> int:
    if type(value) is not int:
        raise SystemExit(f"{field_name} must be an integer")
    if positive and value <= 0:
        raise SystemExit(f"{field_name} must be positive")
    if non_negative and value < 0:
        raise SystemExit(f"{field_name} must be non-negative")
    return value


def validate_review_payload(payload: dict, *, subject: str) -> dict:
    if not isinstance(payload, dict):
        raise SystemExit(f"{subject} must be an object")
    extra = payload.keys() - ALLOWED_TOP_LEVEL_KEYS
    if extra:
        raise SystemExit(f"{subject} has unexpected keys: {sorted(extra)}")
    missing = ALLOWED_TOP_LEVEL_KEYS - payload.keys()
    if missing:
        raise SystemExit(f"{subject} missing required keys: {sorted(missing)}")
    if not isinstance(payload["baseline"], (str, type(None))):
        raise SystemExit(f"{subject} `baseline` must be a string or null")
    if not isinstance(payload["full_scan"], bool):
        raise SystemExit(f"{subject} `full_scan` must be a boolean")
    if not isinstance(payload["summary"], str):
        raise SystemExit(f"{subject} `summary` must be a string")
    if not isinstance(payload["warnings"], list):
        raise SystemExit(f"{subject} `warnings` must be an array")

    for warning in payload["warnings"]:
        if not isinstance(warning, dict):
            raise SystemExit(f"{subject} `warnings` entries must be objects")
        extra = warning.keys() - REQUIRED_WARNING_KEYS
        if extra:
            raise SystemExit(f"{subject} warning has unexpected keys: {sorted(extra)}")
        missing = REQUIRED_WARNING_KEYS - warning.keys()
        if missing:
            raise SystemExit(f"{subject} warning missing required keys: {sorted(missing)}")
        if not isinstance(warning["id"], str):
            raise SystemExit(f"{subject} warning `id` must be a string")
        if not isinstance(warning["category"], str):
            raise SystemExit(f"{subject} warning `category` must be a string")
        if warning["category"] not in ALLOWED_WARNING_CATEGORIES:
            raise SystemExit(
                f"{subject} warning `category` must be one of the allowed categories"
            )
        if not isinstance(warning["severity"], str):
            raise SystemExit(f"{subject} warning `severity` must be a string")
        if warning["severity"] != "warn":
            raise SystemExit(f"{subject} warning `severity` must be `warn`")
        if not isinstance(warning["title"], str):
            raise SystemExit(f"{subject} warning `title` must be a string")
        if not isinstance(warning["why_it_matters"], str):
            raise SystemExit(f"{subject} warning `why_it_matters` must be a string")
        if not isinstance(warning["recommendation"], str):
            raise SystemExit(f"{subject} warning `recommendation` must be a string")
        if not isinstance(warning["evidence"], list):
            raise SystemExit(f"{subject} warning `evidence` must be an array")
        if not warning["evidence"]:
            raise SystemExit(f"{subject} warning `evidence` must not be empty")
        for evidence in warning["evidence"]:
            if not isinstance(evidence, dict):
                raise SystemExit(f"{subject} warning evidence entries must be objects")
            extra = evidence.keys() - {"path", "line", "detail"}
            if extra:
                raise SystemExit(
                    f"{subject} warning evidence has unexpected keys: {sorted(extra)}"
                )
            missing = {"path", "line", "detail"} - evidence.keys()
            if missing:
                raise SystemExit(
                    f"{subject} warning evidence must include `path`, `line`, and `detail`"
                )
            if not isinstance(evidence["path"], str):
                raise SystemExit(f"{subject} warning evidence `path` must be a string")
            if evidence["line"] is not None:
                _require_int(
                    evidence["line"],
                    f"{subject} warning evidence `line`",
                    positive=True,
                )
            if not isinstance(evidence["detail"], str):
                raise SystemExit(f"{subject} warning evidence `detail` must be a string")

    return payload


def validate_review_preview(payload: dict, *, subject: str) -> dict:
    if not isinstance(payload, dict):
        raise SystemExit(f"{subject} must be an object")
    extra = payload.keys() - ALLOWED_PREVIEW_TOP_LEVEL_KEYS
    if extra:
        raise SystemExit(f"{subject} has unexpected keys: {sorted(extra)}")
    missing = ALLOWED_PREVIEW_TOP_LEVEL_KEYS - payload.keys()
    if missing:
        raise SystemExit(f"{subject} missing required keys: {sorted(missing)}")
    if not isinstance(payload["model"], str):
        raise SystemExit(f"{subject} `model` must be a string")
    if not isinstance(payload["review_mode"], str):
        raise SystemExit(f"{subject} `review_mode` must be a string")
    if payload["review_mode"] not in {"default", "fast", "smart"}:
        raise SystemExit(f"{subject} `review_mode` must be `default`, `fast`, or `smart`")
    if not isinstance(payload["codex_invocation"], dict):
        raise SystemExit(f"{subject} `codex_invocation` must be an object")
    if not isinstance(payload["schema_path"], str):
        raise SystemExit(f"{subject} `schema_path` must be a string")
    if not isinstance(payload["backend"], str):
        raise SystemExit(f"{subject} `backend` must be a string")
    if payload["backend"] not in {"jj", "git"}:
        raise SystemExit(f"{subject} `backend` must be `jj` or `git`")
    if not isinstance(payload["baseline"], (str, type(None))):
        raise SystemExit(f"{subject} `baseline` must be a string or null")
    if not isinstance(payload["head"], str):
        raise SystemExit(f"{subject} `head` must be a string")
    if not isinstance(payload["full_scan"], bool):
        raise SystemExit(f"{subject} `full_scan` must be a boolean")
    if not isinstance(payload["changed_files"], list):
        raise SystemExit(f"{subject} `changed_files` must be an array")
    if not all(isinstance(item, str) for item in payload["changed_files"]):
        raise SystemExit(f"{subject} `changed_files` entries must be strings")
    if not isinstance(payload["runner_warnings"], list):
        raise SystemExit(f"{subject} `runner_warnings` must be an array")
    if not all(isinstance(item, str) for item in payload["runner_warnings"]):
        raise SystemExit(f"{subject} `runner_warnings` entries must be strings")
    if not isinstance(payload["review_passes"], list):
        raise SystemExit(f"{subject} `review_passes` must be an array")
    for review_pass in payload["review_passes"]:
        if not isinstance(review_pass, dict):
            raise SystemExit(f"{subject} `review_passes` entries must be objects")
        extra = review_pass.keys() - ALLOWED_PREVIEW_PASS_KEYS
        if extra:
            raise SystemExit(
                f"{subject} review pass has unexpected keys: {sorted(extra)}"
            )
        missing = ALLOWED_PREVIEW_PASS_KEYS - review_pass.keys()
        if missing:
            raise SystemExit(
                f"{subject} review pass missing required keys: {sorted(missing)}"
            )
        if not isinstance(review_pass["name"], str):
            raise SystemExit(f"{subject} review pass `name` must be a string")
        if not isinstance(review_pass["focus"], list) or not all(
            isinstance(item, str) for item in review_pass["focus"]
        ):
            raise SystemExit(f"{subject} review pass `focus` must be a string array")
        if not isinstance(review_pass["files"], list) or not all(
            isinstance(item, str) for item in review_pass["files"]
        ):
            raise SystemExit(f"{subject} review pass `files` must be a string array")
        if not isinstance(review_pass["chunks"], list):
            raise SystemExit(f"{subject} review pass `chunks` must be an array")
        for chunk in review_pass["chunks"]:
            if not isinstance(chunk, dict):
                raise SystemExit(f"{subject} review chunks must be objects")
            extra = chunk.keys() - REQUIRED_PREVIEW_CHUNK_KEYS
            if extra:
                raise SystemExit(
                    f"{subject} review chunk has unexpected keys: {sorted(extra)}"
                )
            missing = REQUIRED_PREVIEW_CHUNK_KEYS - chunk.keys()
            if missing:
                raise SystemExit(
                    f"{subject} review chunk missing required keys: {sorted(missing)}"
                )
            _require_int(
                chunk["chunk_index"],
                f"{subject} review chunk `chunk_index`",
                positive=True,
            )
            _require_int(
                chunk["chunk_count"],
                f"{subject} review chunk `chunk_count`",
                positive=True,
            )
            if chunk["chunk_index"] > chunk["chunk_count"]:
                raise SystemExit(
                    f"{subject} review chunk `chunk_index` must not exceed `chunk_count`"
                )
            if not isinstance(chunk["files"], list) or not all(
                isinstance(item, str) for item in chunk["files"]
            ):
                raise SystemExit(f"{subject} review chunk `files` must be a string array")
            if not isinstance(chunk["diff"], str):
                raise SystemExit(f"{subject} review chunk `diff` must be a string")
            _require_int(
                chunk["estimated_chars"],
                f"{subject} review chunk `estimated_chars`",
                non_negative=True,
            )
            if not isinstance(chunk["file_contexts"], list):
                raise SystemExit(f"{subject} review chunk `file_contexts` must be an array")
            for file_context in chunk["file_contexts"]:
                if not isinstance(file_context, dict):
                    raise SystemExit(
                        f"{subject} review chunk `file_contexts` entries must be objects"
                    )
                extra = file_context.keys() - REQUIRED_FILE_CONTEXT_KEYS
                if extra:
                    raise SystemExit(
                        f"{subject} review chunk file_context has unexpected keys: {sorted(extra)}"
                    )
                missing = REQUIRED_FILE_CONTEXT_KEYS - file_context.keys()
                if missing:
                    raise SystemExit(
                        f"{subject} review chunk file_context missing required keys: {sorted(missing)}"
                    )
                if not isinstance(file_context["path"], str):
                    raise SystemExit(
                        f"{subject} review chunk file_context `path` must be a string"
                    )
                _require_int(
                    file_context["start_line"],
                    f"{subject} review chunk file_context `start_line`",
                    positive=True,
                )
                _require_int(
                    file_context["end_line"],
                    f"{subject} review chunk file_context `end_line`",
                    positive=True,
                )
                if not isinstance(file_context["content"], str):
                    raise SystemExit(
                        f"{subject} review chunk file_context `content` must be a string"
                    )
                if file_context["end_line"] < file_context["start_line"]:
                    raise SystemExit(
                        f"{subject} review chunk file_context `end_line` must be at least `start_line`"
                    )
            if not isinstance(chunk["prompt"], str):
                raise SystemExit(f"{subject} review chunk `prompt` must be a string")

    return payload
