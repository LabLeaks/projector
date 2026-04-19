/**
@spec PROJECTOR.WORKSPACE.PROJECTION_ROOT
Projector materializes synced private context under one or more configured repo-local gitignored projection mounts rather than a hardcoded repo root.

@spec PROJECTOR.WORKSPACE.TEXT_ONLY
v0 materializes only UTF-8 text files and directories under the configured projection mounts.

@spec PROJECTOR.SYNC.TEXT_CONVERGENCE
UTF-8 text files under the configured projection mounts converge across synced checkouts for the same workspace without conflict-marker rewrites on concurrent overlap.

@spec PROJECTOR.SYNC.FILE_LIFECYCLE
The synced workspace reconciles UTF-8 text file creation, body edits, and deletion through a server-backed manifest rather than by treating local disk as the source of truth.

@spec PROJECTOR.PROVENANCE.EVENT_LOG
projector records durable provenance for workspace file lifecycle and body updates with path, timestamp, and operation summary.
*/
