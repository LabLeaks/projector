# @module PROJECTOR.QUALITY.RELEASE_REVIEW.PIPELINE.CONTEXT
# Diff parsing and local code-context extraction for release review. This module does not choose review passes or build prompts.
# @fileimplements PROJECTOR.QUALITY.RELEASE_REVIEW.PIPELINE.CONTEXT
from __future__ import annotations

import re
import subprocess
from pathlib import Path

from release_tooling import run_checked, semver_sort_key

REVIEW_PATHS = ("src", "tests", "scripts", ".github/workflows", "Cargo.toml")
MAX_INPUT_TOKENS = 32_000
CHARS_PER_TOKEN = 4
MAX_INPUT_CHARS = MAX_INPUT_TOKENS * CHARS_PER_TOKEN
MAX_CONTEXT_CHARS = 24_000
NON_RUST_SNIPPET_PADDING = 20
NON_RUST_FULL_SCAN_LINES = 140
RUST_ITEM_RE = re.compile(
    r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:unsafe\s+)?(?:const\s+)?"
    r"(?:extern\s+\"[^\"]+\"\s+)?(?:fn|struct|enum|trait|impl|mod|type|const|static)\b"
)
HUNK_RE = re.compile(r"^@@ -\d+(?:,\d+)? \+(\d+)(?:,(\d+))? @@")
RAW_STRING_START_RE = re.compile(r'(?:br|rb|r)(?P<hashes>#{0,255})"')


def command_exists(name: str) -> bool:
    try:
        subprocess.run(
            [name, "--version"],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=True,
        )
        return True
    except (OSError, subprocess.CalledProcessError):
        return False


def discover_latest_semver_tag(root: Path, backend: str, head: str) -> str:
    if backend == "jj":
        output = run_checked(root, ["jj", "--ignore-working-copy", "tag", "list", "-r", f"::({head})"])
        candidates = [line.split(":", 1)[0].strip() for line in output.splitlines() if ":" in line]
    else:
        output = run_checked(root, ["git", "tag", "--merged", head, "--list"])
        candidates = [line.strip() for line in output.splitlines() if line.strip()]

    versions: list[tuple[tuple[int, int, int, int, tuple[tuple[int, object], ...]], str]] = []
    for candidate in candidates:
        sort_key = semver_sort_key(candidate)
        if sort_key is not None:
            versions.append((sort_key, candidate))

    if not versions:
        if backend == "jj":
            raise SystemExit("no reachable semver release tags found; pass --full or provide --base")
        raise SystemExit(
            f"no reachable semver release tags found from {head}; pass --full or provide --base"
        )

    versions.sort()
    return versions[-1][1]


def changed_files_from_diff(root: Path, backend: str, base: str, head: str) -> list[str]:
    if backend == "jj":
        output = run_checked(
            root,
            [
                "jj",
                "--ignore-working-copy",
                "diff",
                "--name-only",
                "--from",
                base,
                "--to",
                head,
            ],
        )
    else:
        output = run_checked(root, ["git", "diff", "--name-only", base, head, "--", *REVIEW_PATHS])

    files = [line.strip() for line in output.splitlines() if line.strip()]
    return sorted(path for path in files if matches_review_scope(path) and include_file(path))


def matches_review_scope(path: str) -> bool:
    return path == "Cargo.toml" or any(
        path.startswith(prefix.removesuffix("/") + "/") or path == prefix.removesuffix("/")
        for prefix in REVIEW_PATHS
        if prefix != "Cargo.toml"
    )


def list_review_files(root: Path, backend: str) -> list[str]:
    if backend == "jj":
        output = run_checked(root, ["jj", "--ignore-working-copy", "file", "list"])
    else:
        output = run_checked(root, ["git", "ls-files", *REVIEW_PATHS])
    return sorted(
        path for path in output.splitlines() if matches_review_scope(path) and include_file(path)
    )


def full_scan_files(root: Path, backend: str) -> list[str]:
    return list_review_files(root, backend)


def include_file(path: str) -> bool:
    return (
        path.endswith(".rs")
        or path.endswith(".py")
        or path.endswith(".sh")
        or path.endswith(".json")
        or path.endswith(".yml")
        or path.endswith(".yaml")
        or path == "Cargo.toml"
    )


def diff_text_for_paths(root: Path, backend: str, base: str, head: str, paths: list[str]) -> str:
    if not paths:
        return ""
    if backend == "jj":
        return run_checked(
            root,
            [
                "jj",
                "--ignore-working-copy",
                "diff",
                "--git",
                "--context",
                "8",
                "--from",
                base,
                "--to",
                head,
                "--",
                *paths,
            ],
        )
    return run_checked(root, ["git", "diff", "--unified=8", base, head, "--", *paths])


def parse_changed_line_ranges(diff: str) -> dict[str, list[tuple[int, int]]]:
    ranges: dict[str, list[tuple[int, int]]] = {}
    current_path: str | None = None
    current_new_line: int | None = None
    pending_deletion_anchor: int | None = None

    def flush_pending_deletion() -> None:
        nonlocal pending_deletion_anchor
        if current_path is None or pending_deletion_anchor is None:
            return
        ranges.setdefault(current_path, []).append(
            (pending_deletion_anchor, pending_deletion_anchor)
        )
        pending_deletion_anchor = None

    for line in diff.splitlines():
        if line.startswith("diff --git "):
            flush_pending_deletion()
            parts = line.split()
            current_path = parts[3][2:] if len(parts) >= 4 and parts[3].startswith("b/") else None
            current_new_line = None
            continue

        if current_path is None:
            continue

        match = HUNK_RE.match(line)
        if match:
            flush_pending_deletion()
            current_new_line = int(match.group(1))
            continue

        if current_new_line is None:
            continue

        if line.startswith("+++ ") or line.startswith("--- "):
            continue
        if line.startswith("\\"):
            continue

        if line.startswith("+"):
            ranges.setdefault(current_path, []).append((current_new_line, current_new_line))
            current_new_line += 1
            pending_deletion_anchor = None
            continue

        if line.startswith("-"):
            pending_deletion_anchor = max(current_new_line, 1)
            continue

        flush_pending_deletion()
        current_new_line += 1

    flush_pending_deletion()

    return {path: merge_ranges(path_ranges) for path, path_ranges in ranges.items()}


def merge_ranges(ranges: list[tuple[int, int]]) -> list[tuple[int, int]]:
    if not ranges:
        return []
    ordered = sorted(ranges)
    merged = [ordered[0]]
    for start, end in ordered[1:]:
        last_start, last_end = merged[-1]
        if start <= last_end + 1:
            merged[-1] = (last_start, max(last_end, end))
        else:
            merged.append((start, end))
    return merged


def extend_with_attributes(lines: list[str], start_line: int) -> int:
    index = start_line - 2
    while index >= 0:
        stripped = lines[index].strip()
        if stripped.startswith("#[") or stripped.startswith("///") or stripped.startswith("//!"):
            start_line = index + 1
            index -= 1
            continue
        break
    return start_line


def find_rust_item_range(lines: list[str], changed_line: int) -> tuple[int, int] | None:
    max_index = len(lines)
    current = min(max(changed_line, 1), max_index)
    start_line: int | None = None

    for index in range(current - 1, -1, -1):
        if RUST_ITEM_RE.match(lines[index]):
            start_line = extend_with_attributes(lines, index + 1)
            break

    if start_line is None:
        return None

    brace_depth = 0
    saw_block = False
    scan_state = {
        "block_comment_depth": 0,
        "in_string": False,
        "string_escape": False,
        "in_char": False,
        "char_escape": False,
        "raw_string_hashes": None,
    }
    for index in range(start_line - 1, max_index):
        line = lines[index]
        opens, closes = structural_brace_counts(line, scan_state)
        brace_depth += opens
        if opens > 0:
            saw_block = True
        brace_depth -= closes

        if saw_block and brace_depth <= 0:
            return (start_line, index + 1)
        if not saw_block and line.rstrip().endswith(";"):
            return (start_line, index + 1)

    return (start_line, max_index)


def chunk_line_ranges(line_count: int, lines_per_chunk: int) -> list[tuple[int, int]]:
    ranges: list[tuple[int, int]] = []
    start = 1
    while start <= line_count:
        end = min(line_count, start + lines_per_chunk - 1)
        ranges.append((start, end))
        start = end + 1
    return ranges


def extract_top_level_rust_ranges(lines: list[str]) -> list[tuple[int, int]]:
    ranges: list[tuple[int, int]] = []
    index = 0
    while index < len(lines):
        if RUST_ITEM_RE.match(lines[index]):
            item_range = find_rust_item_range(lines, index + 1)
            if item_range is not None:
                ranges.append(item_range)
                index = item_range[1]
                continue
        index += 1
    return merge_ranges(ranges)


def window_range(
    line_count: int, start: int, end: int, padding: int = NON_RUST_SNIPPET_PADDING
) -> tuple[int, int]:
    return (max(1, start - padding), min(line_count, end + padding))


def extract_context_ranges(
    path: str, content: str, line_ranges: list[tuple[int, int]], full_scan: bool
) -> list[tuple[int, int]]:
    lines = content.splitlines()
    line_count = len(lines)
    if line_count == 0:
        return []

    if full_scan:
        return chunk_line_ranges(line_count, NON_RUST_FULL_SCAN_LINES)

    extracted: list[tuple[int, int]] = []
    for start, end in line_ranges:
        if path.endswith(".rs"):
            item_range = find_rust_item_range(lines, start)
            extracted.append(item_range or window_range(line_count, start, end))
        else:
            extracted.append(window_range(line_count, start, end))
    return merge_ranges(extracted)


def structural_brace_counts(line: str, state: dict[str, object]) -> tuple[int, int]:
    opens = 0
    closes = 0
    index = 0

    while index < len(line):
        block_comment_depth = int(state["block_comment_depth"])
        raw_string_hashes = state["raw_string_hashes"]
        if block_comment_depth > 0:
            if line.startswith("/*", index):
                state["block_comment_depth"] = block_comment_depth + 1
                index += 2
                continue
            if line.startswith("*/", index):
                state["block_comment_depth"] = block_comment_depth - 1
                index += 2
                continue
            index += 1
            continue

        if raw_string_hashes is not None:
            hashes = int(raw_string_hashes)
            if line[index] == '"' and line.startswith("#" * hashes, index + 1):
                state["raw_string_hashes"] = None
                index += 1 + hashes
                continue
            index += 1
            continue

        if bool(state["in_string"]):
            if bool(state["string_escape"]):
                state["string_escape"] = False
            elif line[index] == "\\":
                state["string_escape"] = True
            elif line[index] == '"':
                state["in_string"] = False
            index += 1
            continue

        if bool(state["in_char"]):
            if bool(state["char_escape"]):
                state["char_escape"] = False
            elif line[index] == "\\":
                state["char_escape"] = True
            elif line[index] == "'":
                state["in_char"] = False
            index += 1
            continue

        if line.startswith("//", index):
            break
        if line.startswith("/*", index):
            state["block_comment_depth"] = 1
            index += 2
            continue

        raw_match = RAW_STRING_START_RE.match(line[index:])
        if raw_match is not None:
            state["raw_string_hashes"] = len(raw_match.group("hashes"))
            index += raw_match.end()
            continue

        if line[index] == '"':
            state["in_string"] = True
            state["string_escape"] = False
            index += 1
            continue

        if line[index] == "'" and is_char_literal_start(line, index):
            state["in_char"] = True
            state["char_escape"] = False
            index += 1
            continue

        if line[index] == "{":
            opens += 1
        elif line[index] == "}":
            closes += 1
        index += 1

    return opens, closes


def is_char_literal_start(line: str, index: int) -> bool:
    closing = line.find("'", index + 1)
    return closing != -1 and closing - index <= 4


def split_large_context(entry: dict[str, object]) -> list[dict[str, object]]:
    content = str(entry["content"])
    if len(content) <= MAX_CONTEXT_CHARS:
        return [entry]

    lines = content.splitlines()
    if not lines:
        return [entry]

    pieces: list[dict[str, object]] = []
    start_line = int(entry["start_line"])
    current_lines: list[str] = []
    current_chars = 0
    current_start = start_line

    for offset, line in enumerate(lines):
        line_chars = len(line) + 1
        if current_lines and current_chars + line_chars > MAX_CONTEXT_CHARS:
            end_line = current_start + len(current_lines) - 1
            pieces.append(
                {
                    "path": entry["path"],
                    "start_line": current_start,
                    "end_line": end_line,
                    "content": "\n".join(current_lines),
                }
            )
            current_lines = [line]
            current_chars = line_chars
            current_start = start_line + offset
        else:
            current_lines.append(line)
            current_chars += line_chars

    if current_lines:
        end_line = current_start + len(current_lines) - 1
        pieces.append(
            {
                "path": entry["path"],
                "start_line": current_start,
                "end_line": end_line,
                "content": "\n".join(current_lines),
            }
        )

    return pieces


def build_file_contexts(
    root: Path,
    files: list[str],
    changed_line_ranges: dict[str, list[tuple[int, int]]],
    full_scan: bool,
) -> list[dict[str, object]]:
    contexts: list[dict[str, object]] = []
    for relative_path in files:
        path = root / relative_path
        if not path.is_file():
            continue
        content = path.read_text(encoding="utf-8", errors="replace")
        ranges = extract_context_ranges(
            relative_path,
            content,
            changed_line_ranges.get(relative_path, []),
            full_scan,
        )
        lines = content.splitlines()
        for start_line, end_line in ranges:
            snippet = "\n".join(lines[start_line - 1 : end_line])
            contexts.extend(
                split_large_context(
                    {
                        "path": relative_path,
                        "start_line": start_line,
                        "end_line": end_line,
                        "content": snippet,
                    }
                )
            )
    return contexts
