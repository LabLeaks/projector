# @module PROJECTOR.QUALITY.RELEASE_REVIEW.MERGE
# Stable warning deduplication and merged warn-only payload assembly for release review. This module does not invoke Codex or build review prompts.
# @fileimplements PROJECTOR.QUALITY.RELEASE_REVIEW.MERGE
from __future__ import annotations

import hashlib
import json


def merge_pass_responses(
    base: str | None,
    full_scan: bool,
    responses: list[tuple[str, int, dict]],
    runner_warnings: list[str],
) -> dict:
    warnings: list[dict] = []
    seen: set[str] = set()

    ordered_responses = sorted(
        responses,
        key=lambda item: (item[0], item[1], json.dumps(item[2], sort_keys=True)),
    )

    for pass_name, _chunk_index, response in ordered_responses:
        for warning in response.get("warnings", []):
            dedupe_key = warning_dedupe_key(warning)
            if dedupe_key in seen:
                continue
            seen.add(dedupe_key)
            merged = dict(warning)
            merged["id"] = stable_warning_id(pass_name, warning)
            warnings.append(merged)

    warnings.sort(key=warning_sort_key)

    if warnings:
        summary = f"{len(warnings)} warn-level issue(s) found across {len(responses)} review chunk(s)."
    else:
        summary = "No warn-level issues found across release review chunks."
    if runner_warnings:
        summary += f" {len(runner_warnings)} runner warning(s) occurred."

    return {
        "baseline": base,
        "full_scan": full_scan,
        "summary": summary,
        "warnings": warnings,
    }


def warning_dedupe_key(warning: dict) -> str:
    evidence = [
        {
            "path": item.get("path"),
            "line": item.get("line"),
            "detail": item.get("detail"),
        }
        for item in warning.get("evidence", [])
    ]
    evidence.sort(key=lambda item: (item["path"] or "", item["line"] or 0, item["detail"] or ""))
    return json.dumps(
        {
            "category": warning.get("category"),
            "title": warning.get("title"),
            "evidence": evidence,
        },
        sort_keys=True,
    )


def stable_warning_id(pass_name: str, warning: dict) -> str:
    digest = hashlib.sha1(warning_dedupe_key(warning).encode("utf-8")).hexdigest()[:12]
    return f"{pass_name}:{warning['category']}:{digest}"


def warning_sort_key(warning: dict) -> tuple[object, ...]:
    anchors = []
    for item in warning.get("evidence", []):
        anchors.append((item.get("path") or "", item.get("line") or 0, item.get("detail") or ""))
    anchors.sort()
    first_anchor = anchors[0] if anchors else ("", 0, "")
    return (
        warning.get("category", ""),
        first_anchor[0],
        first_anchor[1],
        warning.get("title", ""),
        warning.get("id", ""),
    )
