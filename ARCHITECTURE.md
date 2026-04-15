# ARCHITECTURE

`special modules` is the canonical architecture view for this repo.
This document should explain and summarize the architecture that the source-owned module tree already declares; it should not become a second source of behavioral truth alongside `special specs`.

### `@area PROJECTOR`
projector product architecture.

### `@area PROJECTOR.EDGE`
User-facing command and process entrypoints.

### `@area PROJECTOR.DOMAIN`
Shared ids, manifest, provenance, and wire types.

### `@area PROJECTOR.RUNTIME`
Checkout-local sync coordination and filesystem projection runtime.

### `@area PROJECTOR.SERVER`
Remote API and storage authority for synchronized workspace state.

### `@module PROJECTOR.RUNTIME.BODY_SYNC`
### `@planned`
The body-sync layer reconciles concurrent UTF-8 text edits through per-document CRDT state instead of last-write-wins snapshots.
Until that layer lands, v0 may use deterministic server-side three-way merge with conflict markers as the temporary body-convergence strategy.

## System shape

`projector` has three major layers:

1. workspace manifest
2. per-document body sync
3. durable provenance log

For the first implementation, these layers should be cut to the minimum needed for single-owner multi-machine dogfooding. See [V0.md](/Users/gk/work/lableaks/projects/projector/V0.md).

## Design stance

The current architecture direction is good enough to proceed, but it should be interpreted as a projection system with a disciplined control loop rather than "some files plus some CRDT plus a daemon."

The key stance is:

- the server is authoritative for workspace identity, manifest state, and durable history
- one or more gitignored repo-local mounts are materialized working projections for humans and agents
- `.projector/` is projector-owned local admin state
- the daemon is the only component that should coordinate local filesystem state with remote state

If we keep those boundaries hard, the system should stay understandable.

## Preferred shape

The architecture should settle into three broad strata:

1. edge
2. runtime
3. server

### Edge

The edge layer should stay thin:

- parse CLI arguments
- load repo-local binding and daemon state
- call runtime workflows
- render results, including the restore browser UI

The edge should not contain sync logic, file reconciliation, or protocol decisions.

The current source-owned edge split now follows the real command seams instead of one giant CLI file:

- `PROJECTOR.EDGE.CLI`: top-level dispatch only
- `PROJECTOR.EDGE.CONNECTION_CLI`: thin seam for connection-oriented commands
- `PROJECTOR.EDGE.CONNECTION_PROFILES_CLI`: `connect`, `disconnect`, dependent-entry rendering, and profile selection
- `PROJECTOR.EDGE.DEPLOY_CLI`: remote BYO deploy orchestration over SSH plus sysbox
- `PROJECTOR.EDGE.CONNECTION_PROMPTS`: interactive prompt/default-filling helpers for connection flows
- `PROJECTOR.EDGE.CONNECTION_ARGS`: typed argument parsing for `connect`, `disconnect`, and `deploy`
- `PROJECTOR.EDGE.SYNC_ENTRY_CLI`: `add`, `get`, and `remove`
- `PROJECTOR.EDGE.DAEMON_CLI`: `sync start|stop|status` plus the internal daemon process entrypoint
- `PROJECTOR.EDGE.DIAGNOSTICS_CLI`: diagnostics command seam only
- `PROJECTOR.EDGE.OBSERVABILITY_CLI`: observability seam only
- `PROJECTOR.EDGE.STATUS_CLI`: `status`
- `PROJECTOR.EDGE.DOCTOR_CLI`: `doctor` seam only
- `PROJECTOR.EDGE.DOCTOR_PROFILE_CHECKS`: referenced-profile registration and reachability checks for doctor
- `PROJECTOR.EDGE.DOCTOR_SYNC_ENTRY_CHECKS`: gitignore, tracked-file, and profile-reference checks for doctor
- `PROJECTOR.EDGE.DOCTOR_REPORT`: doctor summary reduction and final rendering
- `PROJECTOR.EDGE.LOG_CLI`: `log` seam only
- `PROJECTOR.EDGE.LOG_LOCAL`: local sync-issue, recovery, and fallback log rendering
- `PROJECTOR.EDGE.LOG_REMOTE`: remote provenance fetch, dedup, and rendering
- `PROJECTOR.EDGE.CONFLICT_SCAN`: projected-text conflict marker scanning used by status
- `PROJECTOR.EDGE.HISTORY_RESTORE_CLI`: history and restore seam only
- `PROJECTOR.EDGE.HISTORY_RESTORE_ARGS`: shared history and restore argument parsing plus selector/defaulting rules
- `PROJECTOR.EDGE.HISTORY_RESTORE_RENDERING`: shared terminal rendering helpers for history, restore, and log surfaces
- `PROJECTOR.EDGE.HISTORY_CLI`: document-history and workspace-history reads
- `PROJECTOR.EDGE.RESTORE_CLI`: restore seam only
- `PROJECTOR.EDGE.RESTORE_PREPARATION`: repo/profile/transport loading and requested-path resolution for restore
- `PROJECTOR.EDGE.RESTORE_SELECTION`: interactive and scripted revision selection for restore
- `PROJECTOR.EDGE.RESTORE_APPLY`: preview rendering, remote restore writes, and local rematerialization

That is a healthier shape because the architectural boundary now matches the user-facing command grammar instead of forcing one file to own unrelated policies.

### Runtime

The runtime is the real client-side system.

It should own:

- repo-local sync-entry configuration
- daemon lifecycle
- local snapshot application
- local file watching
- body-sync translation
- transport session management
- local status derivation

The machine-global runtime control plane is now split more honestly too:

- `PROJECTOR.RUNTIME.GLOBAL_STATE`: thin global-state seam only
- `PROJECTOR.RUNTIME.PROJECTOR_HOME`: projector home discovery and layout
- `PROJECTOR.RUNTIME.SERVER_PROFILES`: machine-global server profile registry
- `PROJECTOR.RUNTIME.MACHINE_SYNC_REGISTRY`: registered repos with sync entries
- `PROJECTOR.RUNTIME.MACHINE_DAEMON_STATE`: daemon pid and heartbeat state
- `PROJECTOR.RUNTIME.MACHINE_DAEMON`: daemon seam only
- `PROJECTOR.RUNTIME.MACHINE_DAEMON_LOOP`: main daemon control loop
- `PROJECTOR.RUNTIME.MACHINE_DAEMON_REPOS`: repo-runtime refresh and watch-root derivation
- `PROJECTOR.RUNTIME.MACHINE_DAEMON_SCHEDULER`: target selection, watcher-event matching, and per-target sync execution

### Server

The server should own durable truth:

- workspace identity
- manifest state
- body state
- provenance log

For single-user v0, the product should assume one or more user-supplied server profiles rather than a local embedded server lane. Each sync entry still has one authoritative server profile at a time, and each sync entry should be a first-class whole remote object rather than a subset attachment. That means the architecture can keep multiple storage or deployment backends internally, but the user-facing path should optimize for binding repos and machines to a real shared authority instead of pretending local-only hosting solves the main problem. Within that, SQLite should be the normal BYO backend and Postgres should be the advanced cloud/PaaS backend.

The server should not care about local filesystem details such as watchdog semantics, debounce quirks, or editor reload behavior.

## 1. Workspace manifest

The manifest is the source of truth for the document set inside one sync entry.

It tracks:

- document id
- path
- kind
- created/deleted state
- rename or move operations
- timestamps

This layer answers questions like:

- what files exist in the configured projection mounts
- what path should each document materialize at
- was this file renamed or deleted

A new text file is not just "new text". It is a new manifest entry plus a new document body.

For v0, the manifest only needs to cover UTF-8 text documents under configured projection mounts. Rename or move support is desirable but should not block the first dogfooding release.

Recommended manifest row shape:

- workspace id
- document id
- relative path under `_project/`
- kind
- live or deleted state
- body version pointer or revision marker
- created at
- updated at

The important design choice is that path identity and body identity are related but not the same thing. A document keeps a stable id even if its materialized path changes later.

Above the manifest, the server should expose a first-class sync-entry object:

- sync-entry id
- current remote root path
- kind: file or folder root
- source repo metadata
- stable relationship to the manifest subtree it owns

Clients should attach or materialize whole sync entries by this stable id. They should not attach arbitrary subsets of an existing remote sync entry.

## 2. Per-document body sync

Each materialized file maps to one canonical document id.

Recommended first design:

- Yjs shared text for document bodies
- local files remain plain text
- sync daemon translates file edits into document updates and remote updates back into file writes

This gives:

- conflict-free convergence for overlapping text edits
- plain local files for agents and humans
- freedom to keep the complex merge model out of the repo itself

The runtime should treat body sync as a document service, not as arbitrary file sync.

Recommended local model:

- each manifest document id maps to one body state handle
- the local file path is a projection target, not the canonical identifier
- watcher edits become body-update intents
- remote body updates become materializer intents

That keeps path churn and body churn from collapsing into one messy codepath.

## 3. Provenance log

Durable provenance should be explicit and append-only.

Each event should record:

- actor id
- machine or agent id
- document id
- timestamp
- operation summary
- optional message or note

For v0, CLI-readable events are sufficient. A polished UI or narrative history can wait.

The exception is restore selection. Picking a restoration point confidently is not a good flags-only workflow, so `projector restore` should use a terminal browser at the edge layer while still delegating history reads and restore writes to the server/runtime boundaries.

This is the out-of-band channel for understanding who changed what and why without collapsing disagreements into git issues.

Live presence is intentionally omitted from v0. In an agentic workflow, "open files" and cursor-style awareness are not central product needs, and basic sync plus provenance should stand on their own.

## Local sync daemon

The intended product shape is one machine-global sync daemon that coordinates many sync-entry attachments across many repo checkouts on that machine.

That daemon:

- loads known repo-local sync-entry configurations
- pulls manifests for the remote entries referenced by those sync entries
- materializes configured projection mounts
- watches local file lifecycle and body edits as the primary local-change signal
- pushes manifest ops and body updates
- applies remote changes back to disk

Its local intake should be event-driven first, with slower filesystem polling and idle remote sweeps used only as backstops rather than tight full-world polling.

The daemon should treat `_project/` as a materialized projection, not the authoritative store.

For v0, the daemon should be optimized for boring correctness:

- survive reconnects
- avoid file write loops
- make recent activity legible via status and log commands

### Daemon control-loop model

The daemon should be designed as one coordinator with explicit internal queues, not as a pile of mutually-calling watchers and network callbacks.

Recommended loop:

1. load known sync-entry configurations and global connection state
2. establish transport sessions for the active server profiles
3. fetch or refresh snapshot baselines for active sync entries
4. apply manifest and body projection to the configured mounts
5. start watcher intake across bound repos
6. serialize local and remote events through one coordinator
7. persist checkpoints that make restart safe

Recommended event classes:

- transport connected or disconnected
- snapshot received
- remote manifest update
- remote body update
- local file changed
- local file created
- local file deleted
- flush or debounce timer fired
- shutdown requested

This should make the daemon easier to reason about than a design where every subsystem writes directly to every other subsystem.

### Write-loop prevention

The daemon should not rely on "hope the watcher ignores our writes."

Preferred pattern:

- materializer writes carry explicit local provenance in runtime state
- watcher intake compares observed changes against recent daemon writes
- the coordinator decides whether an observed change is local-user intent or self-echo

That is less fragile than burying echo suppression inside the watcher alone.

### Projection discipline

The materializer should be a narrow component that maps desired workspace state onto disk.

Prefer this split:

- manifest planner computes desired directory and file operations
- body writer computes content writes for affected document ids
- projection applier performs the filesystem mutations

This makes it easier to test convergence and restart behavior without coupling everything to transport logic.

## Binding and local state

Use `.projector/` for projector-owned state and the configured mounts only for projected user content.

Recommended `.projector/` contents:

- repo-local sync-entry config with local path, remote entry identity, local actor identity, and authoritative server-profile reference
- daemon runtime file with machine-global pid, socket, or lock metadata
- local checkpoint file for last applied remote state
- optional caches that can be discarded and rebuilt

Machine-global connection state should live outside repo bindings.

Recommended global state:

- server-profile registry with named connection targets
- optional SSH metadata and last-known connection coordinates for connected profiles
- last-known health metadata for diagnostics

That keeps `.projector/` focused on checkout-local sync-entry ownership instead of turning every repo into one indivisible remote binding.

The repo-local sync-entry state should be durable enough that one-shot recovery, daemon restart, and path add/remove operations can resume cleanly after a restart.

## Server responsibilities

The server side should stay boring and explicit.

For single-user v0, boring also means operationally boring:

- one authoritative server profile per path-scoped sync entry rather than a pretend local-only authority
- one machine can know about multiple server profiles globally
- repo clients attach through stable named connection profiles instead of retyping raw addresses forever
- reconnect and degraded-network behavior stay understandable
- guided remote deployment can improve setup without changing the sync architecture

## Redesign pressure

The current implementation has moved materially toward the sync-entry model. Repo-local state is now `sync-entries.json`, server profiles are machine-global, the machine daemon drives one runnable sync target per sync entry, and the runtime substrate is file-aware rather than assuming every mount is a directory.

The parts that already fit the new direction reasonably well are:

- manifest entries carrying `mount_relative_path`
- materialization across multiple projection roots
- a machine-global daemon that can coordinate many things
- repo-local path-scoped sync-entry config plus machine-global server profiles
- per-entry daemon work rather than one grouped checkout binding bridge

The parts that likely need redesign are:

- the remaining `CheckoutBinding` compatibility carrier, which still exists in some older runtime and wire code that has not yet been redesigned around sync-entry-native contexts
- transport/bootstrap surfaces, which still assume one workspace id plus one list of projection roots per request
- richer sync-entry discovery and retrieval ergonomics for `projector get`, especially browser polish and later remote-entry browsing at larger scales
- transport reuse and batching, which should eventually group active sync-entry work by server profile for efficiency rather than constructing isolated per-entry transports

So yes: the redesign is real and still underway. The machine daemon is no longer going through a grouped binding bridge, but the last big step is deleting the remaining coarse compatibility carrier entirely rather than continuing to feed older runtime and server surfaces through it.

Recommended service split:

- workspace service: resolve or create workspace identity
- manifest service: canonical file set and path lifecycle
- body service: per-document body state and incremental updates
- provenance service: append-only event retrieval and storage

That split should also be reflected inside the storage backends themselves rather than only in top-level service language. In particular, workspace logic should not stay as one mixed backend blob:

- `PROJECTOR.SERVER.WORKSPACES`: coordination seam for workspace bootstrap, discovery, and delta reads
- `PROJECTOR.SERVER.FILE_WORKSPACES`: file-backed workspace seam
- `PROJECTOR.SERVER.FILE_WORKSPACE_METADATA`: file-backed workspace metadata seam
- `PROJECTOR.SERVER.FILE_WORKSPACE_METADATA_PARSE`: file-backed workspace metadata parsing and sync-entry-kind decoding
- `PROJECTOR.SERVER.FILE_WORKSPACE_METADATA_PERSIST`: file-backed workspace metadata encoding and persistence
- `PROJECTOR.SERVER.FILE_WORKSPACE_BOOTSTRAP`: file-backed bootstrap and changes-since reads
- `PROJECTOR.SERVER.FILE_SYNC_ENTRY_DISCOVERY`: file-backed sync-entry discovery, kind inference, and preview rendering
- `PROJECTOR.SERVER.POSTGRES_WORKSPACES`: Postgres workspace seam
- `PROJECTOR.SERVER.POSTGRES_WORKSPACE_BOOTSTRAP`: Postgres workspace bootstrap and changes-since reads
- `PROJECTOR.SERVER.POSTGRES_SYNC_ENTRY_DISCOVERY`: Postgres sync-entry discovery, kind inference, and preview rendering

Even if these ship as one process and one database at first, keeping the responsibilities explicit will help avoid one giant "sync server" module.

### Server store

The server store should also stay boring and explicit.

The outer store boundary should stay split too:

- `PROJECTOR.SERVER.STORAGE`: storage seam and re-export surface only
- `PROJECTOR.SERVER.STORE_ERROR`: shared storage error model
- `PROJECTOR.SERVER.WORKSPACE_STORE`: async store contract
- `PROJECTOR.SERVER.FILE_STORE`: file-backed store adapter
- `PROJECTOR.SERVER.POSTGRES_STORE`: Postgres store adapter and migration bootstrap

For v0, the right default is SQLite, not a specialized CRDT database.

Use SQLite for the normal single-user BYO path, ideally inside a sysbox-backed container:

- one database file
- easy remote deployment
- easy backup and copy
- transactional enough for v0 manifest, history, restore, and sync-entry discovery

And within that SQLite backend, the code should stay split by responsibility rather than collapsing into one storage blob:

- `PROJECTOR.SERVER.SQLITE_STORAGE`: thin adapter over the store trait
- `PROJECTOR.SERVER.SQLITE_STATE`: schema, workspace rows, and append-only row persistence helpers
- `PROJECTOR.SERVER.SQLITE_WORKSPACES`: bootstrap, sync-entry discovery, and delta reads
- `PROJECTOR.SERVER.SQLITE_MANIFEST`: create/update/delete/move mutations
- `PROJECTOR.SERVER.SQLITE_HISTORY`: event and revision reads plus historical-path lookup
- `PROJECTOR.SERVER.SQLITE_RESTORE`: restore coordination seam
- `PROJECTOR.SERVER.SQLITE_DOCUMENT_RESTORE`: one-document restore seam
- `PROJECTOR.SERVER.SQLITE_DOCUMENT_RESTORE_RESOLUTION`: requested revision lookup, live-entry lookup, and restore-target path validation
- `PROJECTOR.SERVER.SQLITE_DOCUMENT_RESTORE_APPLY`: restored body mutation plus append-only path/body/provenance writes
- `PROJECTOR.SERVER.SQLITE_WORKSPACE_RESTORE`: workspace rewind seam
- `PROJECTOR.SERVER.SQLITE_WORKSPACE_RECONSTRUCTION`: historical workspace snapshot reconstruction
- `PROJECTOR.SERVER.SQLITE_WORKSPACE_RESTORE_PLAN`: restore planning seam
- `PROJECTOR.SERVER.SQLITE_WORKSPACE_RESTORE_DIFF`: current-vs-restored snapshot traversal into per-document restore changes
- `PROJECTOR.SERVER.SQLITE_WORKSPACE_RESTORE_METADATA`: restore metadata seam only
- `PROJECTOR.SERVER.SQLITE_WORKSPACE_RESTORE_CLASSIFICATION`: restore event classification for each planned document change
- `PROJECTOR.SERVER.SQLITE_WORKSPACE_RESTORE_SUMMARY`: summary rendering for each classified restore change
- `PROJECTOR.SERVER.SQLITE_WORKSPACE_RESTORE_APPLY`: application of planned rewind changes onto live workspace state

Use Postgres for the advanced path:

- workspace identity
- configured mount set
- manifest truth
- append-only provenance
- append-only Yjs update blobs plus compacted body snapshots

The key design pattern is:

- SQLite by default for boring self-host
- Postgres when the deployment shape actually needs it
- append-only update storage for document bodies
- periodic compaction for cheap bootstrap

This is a better fit than either:

- pretending body-sync storage should dictate the whole backend architecture
- over-optimizing for a bespoke event store before dogfooding pressure exists

Recommended table families:

- `workspaces`
- `workspace_mounts`
- `documents`
- `document_paths`
- `document_body_updates`
- `document_body_snapshots`
- `provenance_events`

The detailed sketch lives in [STORAGE.md](/Users/gk/work/lableaks/projects/projector/STORAGE.md).

### Snapshot and steady-state model

Prefer snapshot-first boot followed by incremental steady-state updates.

That means:

- bootstrap should return enough state to materialize all configured mounts completely
- steady-state transport should carry smaller manifest and body updates
- reconnect should be able to fall back to a fresh snapshot when local confidence is low

For the server store, that implies:

- bootstrap reads current manifest rows plus compacted body state
- steady-state writes append manifest changes, body updates, and provenance
- compaction is a maintenance concern, not part of the client contract

This is a simpler recovery model than trying to guarantee perfect incremental continuity forever.

## Conflict model

- body conflicts converge at the CRDT layer
- manifest conflicts reconcile at the workspace layer
- local editors may still see "file changed on disk" and require reload or reconcile

That is acceptable for phase 1 as long as convergence is reliable.

### Operational preference

Prefer reliable convergence over clever local UX in v0.

That means it is acceptable if:

- editors sometimes need reload
- rename handling is conservative
- reconnect sometimes reapplies a broader snapshot

It is not acceptable if:

- local content disappears silently
- manifest and body state drift indefinitely
- the daemon cannot explain what it thinks happened

## Patterns to prefer

- single coordinator event loop in the daemon
- explicit boundaries between edge, runtime, and server
- stable document ids beneath materialized paths
- idempotent snapshot application
- append-only provenance events
- small pure planners before side-effectful filesystem writes
- conservative recovery paths that can fall back to full reprojection

## Patterns to avoid

- watcher callbacks writing directly to transport
- transport callbacks writing directly to disk
- storing projector admin state inside `_project/`
- treating path as canonical document identity
- relying on rename detection magic for correctness
- mixing user-facing CLI rendering into runtime coordination logic
- letting provenance become a dumping ground for debug noise instead of durable operator-relevant events

## Boundaries

`projector` is for private strategic workspace sync.

It is not:

- source code sync
- git replacement
- issue tracker
- broad collaboration platform

## Phases

### Phase 1

- private server as source of truth
- local `_project/` mirror
- manifest + per-doc body sync
- plain UTF-8 text files
- basic provenance log

Phase 1 should be treated as the dogfooding v0 release, not just a research milestone.

### Phase 2

- better conflict and reload UX
- optional active-client visibility if real dogfooding shows a need
- comments or discussion threads per document or heading
- stronger editor integration

### Phase 3

- managed cloud service
- richer sharing and policy
- stronger provenance and audit surfaces
