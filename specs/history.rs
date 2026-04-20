/**
@spec PROJECTOR.HISTORY.DOCUMENT_BODY_HISTORY
Projector retains append-only document body revisions with stored retained-body payload and conflict metadata instead of only current body truth.

@spec PROJECTOR.HISTORY.RESTORABLE_WORKSPACE_STATE
Projector can reconstruct workspace state at an earlier workspace cursor as an emergency backstop rather than only retaining current state plus audit events.

@spec PROJECTOR.HISTORY.DOCUMENT_RESTORE
When `projector restore <repo-relative-path>` runs in an interactive terminal, it opens a terminal revision browser and can exit without mutating workspace state.

@spec PROJECTOR.HISTORY.DOCUMENT_RESTORE_PREVIOUS
The terminal revision browser for `projector restore <repo-relative-path>` initially selects the immediately preceding body revision when one exists, so the most likely rollback target is highlighted first.

@spec PROJECTOR.HISTORY.DOCUMENT_RESTORE_CONFIRM
When `projector restore <repo-relative-path>` runs in an interactive terminal, selecting a revision and confirming it from within the browser applies the selected document restore and rematerializes the restored body locally.

@spec PROJECTOR.HISTORY.DOCUMENT_RESTORE_SCRIPTED
`projector restore --seq <seq> <repo-relative-path>` previews the selected document body restoration without mutating workspace state, and adding `--confirm` applies that selected restore non-interactively.

@spec PROJECTOR.HISTORY.DOCUMENT_RESTORE_REVIVES_DELETED_PATH
Interactive `projector restore <repo-relative-path>` can target a document at its last known deleted path and revive it there when that path is still free.

@spec PROJECTOR.HISTORY.DOCUMENT_RESTORE_HISTORICAL_MOVED_PATH
Interactive `projector restore <repo-relative-path>` can target an older moved path from document path history and restore the document back onto that historical path when it is free.

@spec PROJECTOR.HISTORY.MANIFEST_PATH_HISTORY
Projector retains append-only document path history for create, move, and delete transitions rather than only current-path truth.

@spec PROJECTOR.HISTORY.SNAPSHOT_DIFF_HISTORY
Projector renders readable document history as retained text snapshots and diffs over retained body checkpoints instead of exposing raw CRDT update history directly.

@spec PROJECTOR.HISTORY.COMPACTION_POLICY
Projector can attach a path-scoped history compaction policy to one synced path.

@spec PROJECTOR.HISTORY.COMPACTION_POLICY_INHERITANCE
Projector resolves the effective history compaction policy for one file from the nearest configured file or ancestor-folder policy override.

@spec PROJECTOR.HISTORY.CONTENT_REDACTION
Projector can rewrite one document's retained body history by repo-relative path to replace exact matched text with `[REDACTED]` while preserving the document's readable retained history.

@spec PROJECTOR.HISTORY.DOCUMENT_HISTORY_PURGE
Projector can purge one document's retained historical body content by path while leaving the surrounding retained document-body history rows intact.

@spec PROJECTOR.HISTORY.DESTRUCTIVE_HISTORY_AUDIT
Projector records destructive document-history surgery as explicit audit events in its audit trail.

@spec PROJECTOR.SERVER.HISTORY.LISTS_DOCUMENT_BODY_REVISIONS
`POST /history/body/list` returns append-only body revisions for a document with base text, resulting body text, conflict metadata, and self-describing retained-history kind and checkpoint-anchor metadata.

@spec PROJECTOR.SERVER.HISTORY.LISTS_DOCUMENT_PATH_REVISIONS
`POST /history/path/list` returns append-only path revisions for a document with path, delete state, and event kind.

@spec PROJECTOR.SERVER.HISTORY.RESTORES_DOCUMENT_BODY_REVISION
`POST /history/body/restore` restores the selected body revision of a live document as current body state and records a new body revision.

@spec PROJECTOR.SERVER.HISTORY.REVIVES_DELETED_DOCUMENT_AT_LAST_PATH
`POST /history/body/restore` can revive a deleted document at its last stored mount/path when no other live document currently occupies that path.

@spec PROJECTOR.SERVER.HISTORY.RESOLVES_DOCUMENT_BY_HISTORICAL_PATH
`POST /history/path/resolve` resolves a repo-local historical mount/path from document path history to the owning document id, including older moved paths no longer present in current manifest state.

@spec PROJECTOR.SERVER.HISTORY.RECONSTRUCTS_WORKSPACE_AT_CURSOR
`POST /history/workspace/reconstruct` returns the reconstructed workspace manifest and live text bodies for an earlier workspace cursor.

@spec PROJECTOR.SERVER.HISTORY.RESTORES_WORKSPACE_AT_CURSOR
`POST /history/workspace/restore` applies the reconstructed live workspace state for an earlier workspace cursor as current server state.

@spec PROJECTOR.SERVER.HISTORY.RENDERS_SNAPSHOT_DIFF_HISTORY
`POST /history/body/list` returns readable base-to-snapshot diff lines for retained body revisions so clients do not need to interpret raw retained CRDT payloads directly.

@spec PROJECTOR.SERVER.HISTORY.PREVIEWS_REDACTION_MATCHES
`POST /history/body/redact/preview` returns the retained revisions that contain one requested text together with server-rendered redaction previews so clients can browse redaction impact without deriving matches from raw history rows themselves.

@spec PROJECTOR.SERVER.HISTORY.REJECTS_STALE_REDACTION_PREVIEW
`POST /history/body/redact` can require the exact retained revision seq set returned by a prior redaction preview and rejects the rewrite if the matching retained revision set has changed since that preview.

@spec PROJECTOR.SERVER.HISTORY.REDACTS_RETAINED_BODY_HISTORY
`POST /history/body/redact` can rewrite one document's retained body history for a document id by replacing exact matched text with `[REDACTED]` while preserving readable retained history.

@spec PROJECTOR.SERVER.HISTORY.PREVIEWS_PURGE_MATCHES
`POST /history/body/purge/preview` returns the retained revisions whose body content would be cleared by one purge request so clients can preview purge impact without deriving clearable rows from raw history rows themselves.

@spec PROJECTOR.SERVER.HISTORY.REJECTS_STALE_PURGE_PREVIEW
`POST /history/body/purge` can require the exact retained revision seq set returned by a prior purge preview and rejects the purge if the clearable retained revision set has changed since that preview.

@spec PROJECTOR.SERVER.HISTORY.PURGES_DOCUMENT_RETAINED_BODY_HISTORY
`POST /history/body/purge` can purge one document's retained historical body content for a document id while leaving surrounding retained document-body history rows intact.

@spec PROJECTOR.SERVER.HISTORY.RECORDS_DESTRUCTIVE_HISTORY_SURGERY
`POST /history/body/redact|purge` records destructive document-history surgery in the audit record.

@spec PROJECTOR.SERVER.HISTORY.ENFORCES_COMPACTION_POLICY
Server history storage enforces the effective path-scoped history compaction policy during retained body-history writes by keeping dense recent body history and rewriting older retained history as sparser checkpoints.

@spec PROJECTOR.SERVER.HISTORY.REJECTS_INVALID_COMPACTION_POLICY
`POST /history/compact/set` rejects invalid compaction policy values such as zero revisions or zero frequency.

@spec PROJECTOR.SERVER.HISTORY.NORMALIZES_COMPACTION_POLICY_PATHS
`POST /history/compact/get|set` resolves compaction policy overrides by normalized repo-relative path rather than raw path spelling.
*/
