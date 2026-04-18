/**
@spec PROJECTOR.WORKSPACE.PROJECTION_ROOT
Projector materializes synced private context under one or more configured repo-local gitignored projection mounts rather than a hardcoded repo root.

@spec PROJECTOR.WORKSPACE.TEXT_ONLY
v0 materializes only UTF-8 text files and directories under the configured projection mounts.

@spec PROJECTOR.SYNC.TEXT_CONVERGENCE
UTF-8 text files under the configured projection mounts converge across synced checkouts for the same workspace through deterministic server-side three-way merge, preserving both sides with conflict markers when concurrent edits overlap.

@spec PROJECTOR.SYNC.CANONICAL_CRDT_BODY_STATE
@planned
Projector can converge one document's shared body state through a canonical per-document CRDT while keeping repo-local files materialized as normal UTF-8 text.

@spec PROJECTOR.SYNC.CRDT_UPDATE_EXCHANGE
@planned
Projector can synchronize document body changes as CRDT updates against checkpointed body state instead of relying on full-text overwrite attempts and three-way merge on every concurrent edit.

@spec PROJECTOR.SYNC.FILE_LIFECYCLE
The synced workspace reconciles UTF-8 text file creation, body edits, and deletion through a server-backed manifest rather than by treating local disk as the source of truth.

@spec PROJECTOR.PROVENANCE.EVENT_LOG
projector records durable provenance for workspace file lifecycle and body updates with path, timestamp, and operation summary.
*/
