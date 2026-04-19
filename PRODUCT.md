# PRODUCT

## Status

Early design notes for `projector`.

`projector` is a private synced context layer for repo-local strategic files projected into gitignored paths such as `_project/`.

The current design target is a dogfoodable v0 for one owner using multiple repos, machines, and agents. See [V0.md](/Users/gk/work/lableaks/projects/projector/V0.md).

The product is specifically tuned for an agentic development workflow where private project context should stay physically co-located with the code checkout even when it should not enter the repo's public VCS cycle.

## Core problem

Teams want some context to be:

- present inside the repo checkout so agents can read it
- editable as normal files
- independent from the product repo's public commit cycle
- durable and shared across machines and agents
- able to handle overlapping edits without collapsing into git issue churn

A plain gitignored local folder such as `_project/` solves only the first two.

## Product thesis

`projector` should treat repo-local gitignored mounts as synced workspace projections, not the source of truth.

The source of truth should live on private servers first and later a cloud service. For single-user v0, the base case should be one or more user-supplied private server profiles that repos, machines, and agents can all reach. Each sync entry still has exactly one authoritative server profile at a time, and each sync entry is a whole remote object rather than an arbitrary subset. Local projection mounts should be materialized working mirrors that remain editable as plain files.

## User workflow

1. Agent or human checks out the product repo.
2. They start `projector sync start` during setup so the machine-global projector daemon stays on.
3. The configured projection mounts appear locally and stay in sync.
4. They edit normal UTF-8 text files inside those mounts.
5. Changes propagate to other synced workspaces quickly.
6. If another editor changed the file concurrently, local editors may need to reload or reconcile, but the workspace should converge.

## Product requirements

- plain-file local UX
- independent from product repo VCS
- durable shared history
- concurrent editing support
- identity and provenance for changes

For v0, the relevant "editors" are usually the same owner's agents, terminals, scripts, and machines rather than a room of humans with files open interactively.

## Non-goals

- syncing arbitrary source files
- replacing git for product code
- becoming a general collaboration suite before the `_project/` workflow is solid

## Key design choice

`projector` should not rely on dumb file mirroring alone.

It needs both:

- a workspace manifest for file lifecycle
- a document merge model for file bodies

## Research conclusions

### Mutagen-style UX is right

The local experience should feel like a synced working copy.

### Yrs is the current body-level fit

`projector` now uses `yrs` for canonical text-body state so overlapping UTF-8 text edits converge through CRDT-backed document state rather than the old server-side three-way-merge fallback.

### A text CRDT alone is not enough

The body engine does not by itself solve:

- document creation and deletion
- renames and moves
- durable provenance and human-readable edit history

So `projector` still needs a document-set model above per-doc CRDT bodies.

## V0 product cut

The first usable version should optimize for dependable dogfooding, not breadth.

Include:

- one or more repo-local sync-entry attachments per repo projector configuration
- UTF-8 text-file sync into configured gitignored mounts
- local daemon plus server-first state
- one boring bring-your-own private server story for single-user v0

## Single-User Server Story

The implementation can support multiple server storage or deployment modes, but the product story should be narrower than the implementation story.

For single-user v0, the recommended lane should be:

- one or more named private server profiles the user already runs or provisions
- each sync entry points at exactly one authoritative server profile
- each sync entry has one stable server-side sync-entry id and is synced as a whole file or folder root
- profiles are reachable from the repos, machines, and agents that need the shared context
- thin repo-local configuration that registers individual synced paths
- no assumption that a local embedded server adds value by itself
- SQLite as the default single-user BYO store because it keeps remote setup and backups boring
- sysbox-backed container isolation as the default single-user BYO deploy shape so the server stays isolated and future nested-container use stays viable
- Postgres reserved for cloud, PaaS, or more operationally serious deployments

The near-term better-DX lane should be guided deployment to a remote box or PaaS target, and the eventual no-deploy lane can be a managed projector cloud with login, team sharing, and authorization.

### @fileattests PROJECTOR.SERVER.HOSTING.BYO_SERVER
artifact: PRODUCT.md Single-User Server Story and command split describe the shipped named-profile BYO server lane rather than an embedded local-server lane.
owner: gk
last_reviewed: 2026-04-18

This product framing matches the shipped `projector connect` and `projector deploy`
surfaces: users attach repos through named server profiles that refer to
user-supplied shared authorities.

That suggests a clean command split:

- `projector sync start` / `projector sync stop` for machine-global daemon lifecycle
- `projector add` / `projector remove` / `projector rm` for local-first sync-entry attachments under repo projector configuration, with `add` publishing existing local content immediately
- `projector get` for remote-first materialization of an existing whole sync entry by stable id or interactive picker
- `projector connect` for machine-global server profiles, profile selection, and health
- `projector doctor` for explicit repo setup and sync diagnostics without overloading `status`
- `projector deploy` for guided remote provisioning
- managed cloud later, not as a v0 requirement

Exclude:

- arbitrary repo file sync
- rich collaboration features
- presence surfaces
- editor integrations
- binary assets
- team-oriented permission and sharing workflows

History semantics should stay explicit:

- restore is append-only reapplication of prior state, not destructive timeline truncation
- sensitive-revision purge/redaction is a separate destructive history-management feature
- revision retention and compression policy should be configurable per sync entry rather than globally only
