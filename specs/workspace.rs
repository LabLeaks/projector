/**
@spec PROJECTOR.WORKSPACE.PROJECTION_ROOT
Projector materializes synced private context under one or more configured repo-local gitignored projection mounts rather than a hardcoded repo root.

@spec PROJECTOR.WORKSPACE.TEXT_ONLY
projector materializes only UTF-8 text files and directories under the configured projection mounts.

@spec PROJECTOR.SYNC.TEXT_CONVERGENCE
UTF-8 text files under the configured projection mounts converge across synced checkouts for the same workspace without conflict-marker rewrites on concurrent overlap.

@spec PROJECTOR.SYNC.FILE_LIFECYCLE
The synced workspace reconciles UTF-8 text file creation, body edits, and deletion through a server-backed manifest rather than by treating local disk as the source of truth.

@spec PROJECTOR.SYNC.FOLDER_RENAME_PRESERVES_DOCUMENTS
When a local folder under a synced directory mount is renamed, projector treats matching child text files as moved documents and preserves their server-side document ids.

@spec PROJECTOR.SYNC.ROOT_RENAME_PRESERVES_SYNC_ENTRY_BINDINGS
When a configured directory sync-entry root is moved or renamed within the same repo, projector relocates the repo-local sync-entry binding only to a unique gitignored moved root whose files match the previously materialized paths and text fingerprints, instead of treating the old root disappearance as remote document deletion.

@spec PROJECTOR.PROVENANCE.EVENT_LOG
projector records durable provenance for workspace file lifecycle and body updates with path, timestamp, and operation summary.
*/
