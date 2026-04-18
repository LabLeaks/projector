# BODY CRDT DECISION

This note is rationale, not product contract. `specs/` remains the source of truth for product behavior.

## Goal

Choose an off-the-shelf body-state package for `projector` instead of growing a custom CRDT or custom diff/VCS core.

The decision is specifically about canonical document body state for mostly UTF-8 text files projected into a repo-local checkout. It is not a decision to outsource:

- path lifecycle and workspace manifest
- retained history policy
- redact and purge semantics
- server authority and sync-entry scoping
- human-readable diff rendering

## Projector constraints

- Plain text first. `projector` is not trying to ship a browser editor or rich-text model in `0.2.0`.
- Projection-first. `projector` is a system for projecting private synced context into a checkout, not a general collaborative document database.
- Server-backed sync. The server remains the durable authority even if the body engine itself is peer-friendly.
- Checkpointed retained history. We need bounded replay and readable history over checkpoints, not an unbounded forever-oplog.
- Destructive cleanup. Redaction and purge must remain possible over retained history artifacts.
- No browser dependency requirement. Future browser interop is nice, but not a precondition.
- Future non-text assets should be modeled as separate document kinds with blob/snapshot semantics, not shoved through the text body engine.

## Candidates

### `yrs`

Pros:

- Text-oriented CRDT with a mature Rust implementation.
- Built-in update exchange, state vectors, diff updates, and merged update blobs.
- Natural fit for "canonical live body state plus checkpointed retained history".
- Future Yjs interoperability stays available if we ever want JS/browser clients later.

Tradeoffs:

- Retained history and time-travel semantics are not the center of gravity of the package.
- Redact and purge still need projector-owned policy above raw update blobs and snapshots.
- Human-readable diffs still need a separate rendering layer.

### `automerge`

Potential benefit:

- Strong history primitives: heads, changes, historical reads, forks, and diffs are already part of the model.
- History-aware APIs map nicely to "restore older state and diff between points in time".

Why not now:

- The core document model is broader than what `projector` needs.
- For this repo, we would still need to build a narrow text-body layer on top of a more generic object model.
- It pulls the architecture toward a collaborative structured-document system when the product is still "projected context next to code".
- It does not make binary/media files collaborative in any useful sense; those still want their own coarse blob semantics.

## Separate diff layer

Regardless of CRDT choice, readable history diffs should not come from the CRDT package itself. `projector` should render diffs from materialized snapshots and checkpoints using a dedicated diff crate such as `similar` or `imara-diff`.

That keeps the split clean:

- CRDT crate owns convergence and update exchange.
- `projector` owns checkpoints, retained history policy, and destructive cleanup.
- Diff crate owns human-readable presentation.

## Decision

Use `yrs` as the preferred canonical text-body engine.

Reasons:

- `projector` is projection-first and mostly text, and `yrs` is closer to that needed shape.
- The update/state-vector model fits the server-backed checkpoint architecture we already want.
- It gives us a serious CRDT core without forcing `projector` into a more general collaborative document-database model.
- It preserves an escape hatch to browser/JS interop later without making that a requirement now.
- It leaves room for future non-text document kinds to use different storage semantics without overloading one broad CRDT model as the answer to every file type.

Do not treat `yrs` update blobs as the user-facing history format. The retained-history layer should stay projector-owned and checkpoint-oriented.

## Rejected direction

Do not continue building a custom body-convergence engine beyond the temporary seam work already landed. The new storage seams should become the integration boundary for a real CRDT package, not the foundation of a homegrown VCS/CRDT.

## Expected implementation shape

- `yrs` text state becomes the canonical live representation for `DocumentKind::Text`.
- Server sync exchanges encoded updates plus checkpoints.
- Retained history stores checkpoints and enough metadata to render readable diffs.
- Redact and purge operate on retained checkpoints and retained update artifacts, not directly on live projected files.
- Future binary or image-like document kinds should use separate snapshot/blob semantics rather than the text CRDT path.

## Next step

Run one narrow spike that proves:

- one document can be materialized from `yrs` state to normal UTF-8 text
- concurrent updates converge without conflict markers
- server storage can checkpoint and reload the body state
- readable snapshot diffs can still be rendered from retained checkpoints
