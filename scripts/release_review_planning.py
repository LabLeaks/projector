# @module PROJECTOR.QUALITY.RELEASE_REVIEW.PIPELINE.PLANNING
# Review-pass selection, prompt construction, and chunk budgeting for release review. This module does not parse diffs or extract source snippets on its own.
# @fileimplements PROJECTOR.QUALITY.RELEASE_REVIEW.PIPELINE.PLANNING
from __future__ import annotations

from pathlib import Path

from release_review_context import MAX_INPUT_CHARS, diff_text_for_paths

MAX_CONCURRENT_REVIEW_CHUNKS = 3
REVIEW_PASSES = (
    {
        "name": "server_runtime",
        "focus": [
            "state-model safety, retained-history invariants, and low-level layering inside server and runtime implementation paths",
            "whether storage, transport, and projection logic stay cleanly separated without critiquing intended product semantics",
        ],
        "prefixes": ("src/server/", "src/runtime/", "src/domain/"),
        "files": (),
    },
    {
        "name": "cli_surfaces",
        "focus": [
            "implementation quality of CLI boundary code, including side effects, terminal flows, error handling, and low-level factoring within command paths",
            "whether command-boundary tests prove stable observable outcomes instead of helper internals",
        ],
        "prefixes": ("src/cli/", "tests/local_bootstrap/"),
        "files": (
            "tests/local_bootstrap.rs",
            "tests/postgres_bootstrap.rs",
        ),
    },
    {
        "name": "release_distribution",
        "focus": [
            "release script robustness, workflow correctness, exact asset sets, and stable artifact identity selection",
            "whether distribution checks fail on missing required members instead of validating only what happens to be present",
        ],
        "prefixes": (".github/workflows/",),
        "files": (
            "Cargo.toml",
            "tests/release_pipeline.rs",
            "scripts/release-assets.json",
            "scripts/review-projector-release-style.py",
            "scripts/update-homebrew-formula.py",
            "scripts/verify-homebrew-formula.sh",
            "scripts/verify-github-release-published.sh",
            "scripts/tag-release.py",
        ),
    },
    {
        "name": "quality_tooling",
        "focus": [
            "review-wrapper correctness, local-only enforcement, prompt budgeting, and chunking behavior",
            "whether the release-quality machinery itself is robust enough to trust during tagging",
        ],
        "prefixes": (),
        "files": (
            "scripts/review-projector-release-style.py",
            "scripts/release_review_pipeline.py",
            "scripts/release_review_context.py",
            "scripts/release_review_planning.py",
            "scripts/release_review_contract.py",
            "scripts/release_review_invoke.py",
            "scripts/release_review_merge.py",
            "scripts/projector-release-review.schema.json",
            "tests/quality_release_review.rs",
            "tests/support/quality.rs",
        ),
    },
)

SCOPE_RULES = (
    "This is an implementation-quality review of code changes, not a product review, spec review, or architecture review.",
    "Ignore product specs, architecture docs, README wording, changelog wording, and whether code matches external spec or architecture documents.",
    "Only report issues that require engineering judgment and are not normally caught by clippy.",
    "Focus on implementation quality: type design, invariants, error handling, local layering, low-level factoring inside module boundaries, test quality, maintainability, and release risk.",
    "Assume intended product behavior and intended architecture are correct inputs unless the implementation itself is internally contradictory or dangerously unsound.",
    "Do not perform a spec review or an architecture review.",
    "Do not recommend different product semantics, command semantics, or module-tree intent unless the code itself contains an implementation contradiction, invalid state, or unsafe behavior.",
    "You may flag low-level design issues inside the changed code, such as duplicated logic, hidden ambient coupling, weak transaction boundaries, or mixed responsibilities within a local implementation path.",
    "Review only the provided code, tests, release scripts, workflow snippets, and Cargo metadata as code.",
    "Prefer findings that are likely to matter after refactors, not transient nits.",
    "Judge the net current code in the provided context. Do not report deleted-file coverage loss if equivalent or stronger current coverage is also present in the supplied changed files.",
    "Flag loose existence or prefix checks when the real contract is an exact set, exact shape, or synchronized generated artifact.",
    "For validator scripts, treat 'missing required member is silently unvalidated' as a real defect even when present entries are checked correctly.",
    "Flag codepaths where the implementation of the same feature disagrees across modes, flags, or output backends.",
    "Flag positional assumptions when stable identity selection would make the code more robust.",
    "Flag help or informational commands that do surprising discovery, warnings, or side effects unrelated to the request.",
    "Flag invalid sentinel values or contradictory states that should be rejected at the boundary or encoded in types.",
    "Prefer tests that prove observable outcomes, exact contract shape, and user-visible behavior over helper structure or incidental file layout.",
    "Do not infer missing test/help coverage from removed files alone when the provided current files show replacement coverage for the same changed surface.",
    "When the context only includes local snippets, do not speculate about unseen repository areas or missing integration that is not evidenced in the provided diff/snippets.",
    "Do not report formatting trivia, micro-style nits, or issues already well-covered by clippy.",
)

PROJECT_REVIEW_RULES = (
    "CLI orchestration, runtime coordination, and server authority should stay cleanly layered instead of sharing ad hoc branches.",
    "Retained history and compaction machinery should preserve exact user-visible restore and audit behavior even when internal representation changes.",
    "Diagnostics are user-facing product behavior and should preserve actionable context.",
    "Panic paths are only appropriate for internal invariants, not normal user or filesystem failure paths.",
    "The wrapper prefiltered this review to changed files or local syntax-aware snippets; judge the code shown rather than asking for unrelated repository context.",
)


def pass_matches_file(path: str, pass_config: dict[str, object]) -> bool:
    prefixes = pass_config["prefixes"]
    files = pass_config["files"]
    return path in files or any(path.startswith(prefix) for prefix in prefixes)


def build_review_passes(files: list[str]) -> list[dict[str, object]]:
    passes: list[dict[str, object]] = []
    covered: set[str] = set()

    for pass_config in REVIEW_PASSES:
        pass_files = [path for path in files if pass_matches_file(path, pass_config)]
        if not pass_files:
            continue
        covered.update(pass_files)
        passes.append(
            {
                "name": pass_config["name"],
                "focus": list(pass_config["focus"]),
                "files": pass_files,
            }
        )

    unmatched_files = [path for path in files if path not in covered]
    if unmatched_files:
        passes.append(
            {
                "name": "default",
                "focus": [
                    "public API shape, invariants, layering, test quality, and release risk in the provided changed code",
                ],
                "files": unmatched_files,
            }
        )

    return passes


def build_prompt(
    version: str,
    backend: str,
    base: str | None,
    full_scan: bool,
    pass_name: str,
    pass_focus: list[str],
    chunk_files: list[str],
    diff: str,
    file_contexts: list[dict[str, object]],
) -> str:
    mode = "full scan" if full_scan else "diff review"
    prompt = [
        "Review the provided projector release code context and return structured warn-only findings.",
        "",
        "Scope rules:",
    ]

    prompt.extend(f"- {rule}" for rule in SCOPE_RULES)
    prompt.extend(["", "Project-specific review rules:"])
    prompt.extend(f"- {rule}" for rule in PROJECT_REVIEW_RULES)
    prompt.extend(
        [
            "",
            "Return JSON matching the provided schema. If there are no substantive issues, return an empty warnings array.",
            "",
        ]
    )
    prompt.extend(
        [
            f"Current package version: {version}",
            f"Review backend: {backend}",
            f"Review mode: {mode}",
            f"Review pass: {pass_name}",
            f"Baseline: {base or '<none>'}",
            "",
            "Pass focus:",
        ]
    )

    prompt.extend(f"- {item}" for item in pass_focus)
    prompt.extend(["", "Changed files:"])
    if chunk_files:
        prompt.extend(f"- {path}" for path in chunk_files)
    else:
        prompt.append("- <none>")

    prompt.append("")
    prompt.extend(["<diff>", diff or "<no diff>", "</diff>", "", "<file_contexts>"])
    for entry in file_contexts:
        prompt.extend(
            [
                (
                    f"<file path=\"{entry['path']}\" "
                    f"start_line=\"{entry['start_line']}\" end_line=\"{entry['end_line']}\">"
                ),
                str(entry["content"]),
                "</file>",
                "",
            ]
        )
    prompt.append("</file_contexts>")
    return "\n".join(prompt)


def estimate_chars(text: str) -> int:
    return len(text)


def build_pass_chunks(
    root: Path,
    version: str,
    backend: str,
    base: str | None,
    head: str,
    full_scan: bool,
    review_pass: dict[str, object],
    file_contexts: list[dict[str, object]],
) -> tuple[list[dict[str, object]], list[str]]:
    runner_warnings: list[str] = []
    chunks: list[dict[str, object]] = []
    diff_cache: dict[tuple[str, ...], str] = {}

    def cached_diff(paths: list[str]) -> str:
        if full_scan or base is None or not paths:
            return ""
        key = tuple(paths)
        if key not in diff_cache:
            diff_cache[key] = diff_text_for_paths(root, backend, base, head, paths)
        return diff_cache[key]

    if not file_contexts:
        diff = cached_diff(list(review_pass["files"]))
        prompt = build_prompt(
            version,
            backend,
            base,
            full_scan,
            str(review_pass["name"]),
            list(review_pass["focus"]),
            list(review_pass["files"]),
            diff,
            [],
        )
        estimated_chars = estimate_chars(prompt)
        if estimated_chars > MAX_INPUT_CHARS:
            runner_warnings.append(
                f"skipped pass {review_pass['name']}: prompt exceeded {MAX_INPUT_CHARS} chars with no chunkable context"
            )
            return ([], runner_warnings)
        return (
            [
                {
                    "name": review_pass["name"],
                    "focus": review_pass["focus"],
                    "chunk_index": 1,
                    "chunk_count": 1,
                    "files": review_pass["files"],
                    "file_contexts": [],
                    "diff": diff,
                    "prompt": prompt,
                    "estimated_chars": estimated_chars,
                }
            ],
            runner_warnings,
        )

    current_contexts: list[dict[str, object]] = []

    def finalize_chunk(contexts: list[dict[str, object]]) -> None:
        chunk_files = sorted({str(entry["path"]) for entry in contexts})
        diff = cached_diff(chunk_files)
        prompt = build_prompt(
            version,
            backend,
            base,
            full_scan,
            str(review_pass["name"]),
            list(review_pass["focus"]),
            chunk_files,
            diff,
            contexts,
        )
        chunks.append(
            {
                "name": review_pass["name"],
                "focus": review_pass["focus"],
                "files": chunk_files,
                "file_contexts": contexts,
                "diff": diff,
                "prompt": prompt,
                "estimated_chars": estimate_chars(prompt),
            }
        )

    for entry in file_contexts:
        trial_contexts = current_contexts + [entry]
        chunk_files = sorted({str(item["path"]) for item in trial_contexts})
        diff = cached_diff(chunk_files)
        prompt = build_prompt(
            version,
            backend,
            base,
            full_scan,
            str(review_pass["name"]),
            list(review_pass["focus"]),
            chunk_files,
            diff,
            trial_contexts,
        )

        if estimate_chars(prompt) <= MAX_INPUT_CHARS:
            current_contexts = trial_contexts
            continue

        if current_contexts:
            finalize_chunk(current_contexts)
            current_contexts = [entry]
        else:
            current_contexts = [entry]

        single_files = [str(entry["path"])]
        single_diff = cached_diff(single_files)
        single_prompt = build_prompt(
            version,
            backend,
            base,
            full_scan,
            str(review_pass["name"]),
            list(review_pass["focus"]),
            single_files,
            single_diff,
            current_contexts,
        )
        if estimate_chars(single_prompt) > MAX_INPUT_CHARS:
            runner_warnings.append(
                f"skipped {review_pass['name']} chunk for {entry['path']}:{entry['start_line']}-{entry['end_line']}: "
                f"estimated {estimate_chars(single_prompt)} chars exceeds {MAX_INPUT_CHARS} char budget"
            )
            current_contexts = []

    if current_contexts:
        finalize_chunk(current_contexts)

    covered_paths = {
        path
        for chunk in chunks
        for path in chunk["files"]
        if chunk["file_contexts"]
    }
    has_current_test_context = any(
        str(entry["path"]).startswith("tests/") for entry in file_contexts
    )
    diff_only_files = [
        path
        for path in review_pass["files"]
        if path not in covered_paths
        and not (has_current_test_context and path.startswith("tests/"))
    ]

    if diff_only_files:
        diff = cached_diff(diff_only_files)
        prompt = build_prompt(
            version,
            backend,
            base,
            full_scan,
            str(review_pass["name"]),
            list(review_pass["focus"]),
            diff_only_files,
            diff,
            [],
        )
        estimated_chars = estimate_chars(prompt)
        if estimated_chars > MAX_INPUT_CHARS:
            runner_warnings.append(
                f"skipped pass {review_pass['name']} deleted-file diff chunk: prompt exceeded {MAX_INPUT_CHARS} chars"
            )
        else:
            chunks.append(
                {
                    "name": review_pass["name"],
                    "focus": review_pass["focus"],
                    "files": diff_only_files,
                    "file_contexts": [],
                    "diff": diff,
                    "prompt": prompt,
                    "estimated_chars": estimated_chars,
                }
            )

    for index, chunk in enumerate(chunks, start=1):
        chunk["chunk_index"] = index
        chunk["chunk_count"] = len(chunks)

    return (chunks, runner_warnings)
