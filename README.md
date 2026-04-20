# projector

`projector` is a private synced-workspace system for repo-local working context projected into gitignored files and folders such as `AGENTS.md`, `notes/`, `briefs/`, or a private workspace directory.

The goal is to let agents and humans read and edit plain files inside repo-local gitignored paths while the real source of truth lives elsewhere and syncs across checkouts.

## What's New In 0.2.0

`0.2.0` is the first real public step beyond initial dogfood.

This release turns `projector` from durable private file sync into a serious private shared-context utility for repo-local text:

- concurrent UTF-8 text edits now converge through CRDT-backed text state rather than the old server-side three-way-merge fallback
- retained history is now a first-class feature with readable snapshot-and-diff history, restore, redact, and purge
- retained history can compact under server-owned path policy without entering the repo's public Git history
- the CLI now exposes explicit history-surgery and compaction flows: `projector redact`, `projector purge`, and `projector compact`

## Why

Keeping private working files inside a product repo, but outside version control, has two good properties:

- it keeps private strategy out of the product repo's public VCS cycle
- it keeps the files physically present so agents can read them

That split matters:

- it needs to live in the repo because the repo is where agents and humans are already working, searching, editing, and reasoning
- it cannot live in normal VCS because these files are often private, noisy, high-churn, machine-local, or simply not meant to become part of the shared product history

But it also has two bad properties:

- local state is fragile and easy to lose
- there is no durable shared history or concurrent-edit model

`projector` exists to solve that without forcing you into a separate notes app, a second repo, or fragile local-only state.

`projector` does not try to solve general agent memory, prompt optimization, or automatic context compression. It solves a narrower problem: keep private working files physically near the repo where agents operate, but durable, cross-machine, restorable, and outside normal Git history.

In practice, that means workflows like:

- keeping a repo-local `AGENTS.md` with project-specific guidance that should stay visible to local agents but out of the shared product history
- keeping private notes, briefs, research, and backlog files next to the code so agents can read and update them during normal work
- carrying that working context across laptop, desktop, and remote machines without manually copying gitignored files around
- being able to inspect history and restore prior working state when a private file gets mangled, overwritten, or changed in the wrong direction

For example, a repo might look like:

```text
my-app/
├── src/
├── tests/
├── AGENTS.md                # private repo-local agent guidance
├── notes/
│   ├── inbox.md
│   └── decisions.md
├── briefs/
│   └── launch-plan.md
├── research/
│   └── competitors.html
└── .projector/
```

Those files stay physically present in the repo so agents can read and edit them, but they do not need to become part of the normal product commit history.

The canonical product contract lives under [specs/](/Users/gk/work/lableaks/projects/projector/specs/root.md) and should be inspected with `special specs`. Root docs remain framing and design context unless a claim is explicitly carried into the contract.

## Product shape

- private working files stay physically present in a repo checkout, but outside the repo's public VCS history
- the canonical source of truth lives on a user-supplied server profile rather than a local embedded server
- the normal single-user BYO path is `projector` plus a remote `projector-server`, with SQLite as the default store and sysbox-isolated deploy as the default deploy shape
- Postgres remains the advanced store for cloud, PaaS, or managed-service environments
- each sync entry is one authoritative remote object with one active server profile at a time
- a machine-global sync daemon is the normal always-on mode
- each repo checkout materializes one or more local gitignored projection mounts
- `projector add` is local-first and bootstraps local content immediately; `projector get` is remote-first
- agents still edit normal local files
- the daemon pushes and pulls changes continuously
- concurrent UTF-8 text edits reconcile through CRDT-backed text state
- retained history supports readable history, restore, redact, purge, and path-scoped compaction policy without entering the repo's public Git history

## Scope

Initial scope is only gitignored private-context files and folders such as:

- `AGENTS.md`
- text notes
- inbox
- product strategy
- backlog
- research
- HTML
- source files

It is not a general-purpose repo file sync system.

## Current design docs

- [PRODUCT.md](/Users/gk/work/lableaks/projects/projector/PRODUCT.md)
- [ARCHITECTURE.md](/Users/gk/work/lableaks/projects/projector/ARCHITECTURE.md)
- [SLICE1.md](/Users/gk/work/lableaks/projects/projector/SLICE1.md)
- [specs/](/Users/gk/work/lableaks/projects/projector/specs/root.rs)

## Install

The intended local install surface for the current `0.2.1` release is the `projector` CLI:

```sh
brew install LabLeaks/homebrew-tap/projector
```

Versioned GitHub Releases also publish release archives for both:

- `projector`
- `projector-server`

This release does not use crates.io as a supported distribution surface.

In the normal BYO-server flow, users do not manually install `projector-server` on the remote machine. `projector deploy` is responsible for provisioning that binary into the remote sysbox-isolated runtime.

## Release

`projector` now carries its own local release wrapper and GitHub release automation.

Release notes for the current public release live in [CHANGELOG.md](/Users/gk/work/lableaks/projects/projector/CHANGELOG.md).

Run the local release wrapper with:

```sh
python3 scripts/tag-release.py X.Y.Z
```

The wrapper confirms the prerelease checklist, pushes `main`, pushes the release tag, waits for the GitHub Release assets to publish, then updates and verifies the Homebrew formula in `LabLeaks/homebrew-tap`.
