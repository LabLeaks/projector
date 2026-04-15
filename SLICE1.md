# Slice 1

This document explains the first implementation slice in prose.

The canonical product contract remains in [specs/](/Users/gk/work/lableaks/projects/projector/specs/root.rs) and the canonical architecture tree remains in `special modules`.

## Goal

Slice 1 should make one checkout usable with a real remote source of truth.

That means:

- `projector sync` can bind the current checkout to one workspace
- projector can materialize one or more configured repo-local mounts from a remote snapshot
- a local daemon can keep that projection up to date
- `projector status` and `projector log` can make the system legible

This slice does not need to solve multi-machine attachment ergonomics yet.

## Chosen cut

### Local state

Use a hidden repo-local `.projector/` directory for projector-owned state.

That directory should hold:

- checkout binding metadata
- daemon runtime metadata such as pid or socket details if needed
- local caches or checkpoints that help restart safely

Configured projection mounts should stay reserved for materialized user content only.

### Daemon model

Start foreground-first.

`projector sync` should be the primary one-shot entrypoint that binds and materializes the current checkout when needed. `projector sync --watch` should be the primary long-running foreground runtime for the current checkout. Background service installation can wait until the single-checkout loop is dependable.

Internally, prefer one daemon coordinator that serializes:

- snapshot application
- watcher intake
- remote update intake
- outbound flush scheduling

Avoid a design where watcher code, materializer code, and transport code all mutate shared state independently.

### Snapshot-first sync

The first sync path should be:

1. resolve or create the remote workspace binding
2. fetch the current manifest snapshot
3. fetch current document bodies for manifest entries
4. materialize the configured mounts
5. enter steady-state update flow

This keeps initial correctness simpler than trying to begin from watchers and incremental updates alone.

### Local runtime pattern

The first runtime cut should likely separate:

- binding store
- daemon coordinator
- manifest planner
- body sync adapter
- materializer
- watcher adapter
- transport adapter
- provenance reader

That is more components than the final binary surface, but the boundaries are useful because they reflect real responsibilities rather than framework taste.

### Initial Rust workspace layout

The first code layout should mirror those boundaries directly:

- `src/domain`: shared ids and manifest or provenance domain types
- `src/runtime`: binding store, daemon coordinator, materializer, watcher, and transport surfaces
- `src/cli`: thin edge binary
- `src/server`: server binary and service composition root

This is a workspace-shape decision, not a forever packaging commitment.

### Minimal server surfaces

The first server contract only needs a few capabilities:

- resolve workspace identity for the current checkout
- return a manifest snapshot for a workspace
- return current bodies for manifest entries
- accept local updates
- return durable provenance events

The exact transport shape can stay flexible during implementation as long as those capabilities exist.

Current implementation direction:

- minimal `axum` HTTP server edge
- JSON bootstrap request and response payloads
- file-backed workspace metadata on the server side until the first Postgres-backed store lands

That is still intentionally small, but it is a more maintainable edge than raw hand-rolled socket parsing.

### First real server store

The next storage cut should be standard, not specialized:

- Postgres as the primary server store
- SQL truth for workspaces, mounts, manifest rows, and provenance
- append-only Yjs update blobs plus compacted body snapshots for document bodies

Avoid introducing a CRDT-specific database in Slice 1. The product needs ordinary transactional correctness more than it needs a novel persistence layer.

### Recovery preference

Slice 1 should bias toward safe reprojection.

If local runtime confidence is low after reconnect, crash recovery, or version mismatch, the daemon should prefer:

1. refetch snapshot
2. recompute desired projection
3. rewrite the configured mounts toward the desired state

That is preferable to trying to preserve a dubious incremental state machine.

## Deferred decisions

These stay intentionally open after Slice 1:

- how a second machine binds to an existing workspace
- whether daemon state is represented by a pid file, socket, lock, or another mechanism
- whether the steady-state transport is WebSocket-first or another long-lived stream
- whether rename handling is explicit in Slice 1 or deferred to a later pass

## Why this slice first

This is the smallest cut that proves projector is a real system instead of a docs idea.

If Slice 1 works, the repo can start dogfooding the local admin shape, projection-mount semantics, daemon loop, and the distinction between projector-owned state and user-owned project context.
