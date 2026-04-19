/**
@spec PROJECTOR.HISTORY.DOCUMENT_BODY_HISTORY
Projector retains append-only document body revisions with base text, resulting body text, and conflict metadata instead of only current body truth.

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
`projector restore <repo-relative-path> --confirm` can target a document at its last known deleted path and revive it there when that path is still free.

@spec PROJECTOR.HISTORY.DOCUMENT_RESTORE_HISTORICAL_MOVED_PATH
`projector restore <repo-relative-path> --confirm` can target an older moved path from document path history and restore the document back onto that historical path when it is free.

@spec PROJECTOR.HISTORY.MANIFEST_PATH_HISTORY
Projector retains append-only document path history for create, move, and delete transitions rather than only current-path truth.

@spec PROJECTOR.HISTORY.SNAPSHOT_DIFF_HISTORY
Projector renders readable document history as retained text snapshots and diffs over retained body checkpoints instead of exposing raw CRDT update history directly.

@spec PROJECTOR.HISTORY.COMPACTION_POLICY
@planned
Projector can attach a path-scoped history compaction policy that keeps recent document body history at full fidelity and older document body history as sparser retained checkpoints.

@spec PROJECTOR.HISTORY.COMPACTION_POLICY_INHERITANCE
@planned
Projector resolves the effective history compaction policy for one file from the nearest configured file or ancestor-folder policy override.

@spec PROJECTOR.HISTORY.DEFAULT_COMPACTION_POLICY
@planned
Projector keeps the most recent 100 document body revisions at full fidelity and retains 1 checkpoint out of every 10 older revisions unless a nearer path policy overrides that default.

@spec PROJECTOR.HISTORY.CONTENT_REDACTION
Projector can rewrite one document's retained checkpoint and update history by repo-relative path to replace exact matched text with `[REDACTED]` while preserving the document's readable retained history.

@spec PROJECTOR.HISTORY.DOCUMENT_HISTORY_PURGE
Projector can purge one document's retained historical body content by path without deleting the surrounding non-secret audit record that history surgery happened.

@spec PROJECTOR.HISTORY.DESTRUCTIVE_HISTORY_AUDIT
Projector records destructive document-history surgery durably without retaining the removed sensitive content in its audit trail.

@spec PROJECTOR.SERVER.HISTORY.LISTS_DOCUMENT_BODY_REVISIONS
`POST /history/body/list` returns append-only body revisions for a document with base text, resulting body text, and conflict metadata.

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

@spec PROJECTOR.SERVER.HISTORY.CHECKPOINTS_CDRT_BODY_STATE
@planned
Server history storage can retain checkpointed document body snapshots together with CRDT update history so older dense update runs can compact without losing readable history reconstruction.

@spec PROJECTOR.SERVER.HISTORY.RENDERS_SNAPSHOT_DIFF_HISTORY
`POST /history/body/list` returns readable base-to-snapshot diff lines for retained body revisions so clients do not need to interpret raw retained CRDT payloads directly.

@spec PROJECTOR.SERVER.HISTORY.PREVIEWS_REDACTION_MATCHES
`POST /history/body/redact/preview` returns the retained revisions that exactly match one requested text together with server-rendered redaction previews so clients can browse redaction impact without deriving matches from raw history rows themselves.

@spec PROJECTOR.SERVER.HISTORY.REJECTS_STALE_REDACTION_PREVIEW
`POST /history/body/redact` can require the exact retained revision seq set returned by a prior redaction preview and rejects the rewrite if the matching retained revision set has changed since that preview.

@spec PROJECTOR.SERVER.HISTORY.REDACTS_RETAINED_BODY_HISTORY
`POST /history/body/redact` can rewrite one document's retained checkpoints and update history for a document id by replacing exact matched text with `[REDACTED]` while preserving readable retained history.

@spec PROJECTOR.SERVER.HISTORY.PREVIEWS_PURGE_MATCHES
`POST /history/body/purge/preview` returns the retained revisions whose body content would be cleared by one purge request so clients can preview purge impact without deriving clearable rows from raw history rows themselves.

@spec PROJECTOR.SERVER.HISTORY.REJECTS_STALE_PURGE_PREVIEW
`POST /history/body/purge` can require the exact retained revision seq set returned by a prior purge preview and rejects the purge if the clearable retained revision set has changed since that preview.

@spec PROJECTOR.SERVER.HISTORY.PURGES_DOCUMENT_RETAINED_BODY_HISTORY
`POST /history/body/purge` can purge one document's retained historical body content for a document id without deleting the surrounding non-secret audit record that history surgery happened.

@spec PROJECTOR.SERVER.HISTORY.RECORDS_DESTRUCTIVE_HISTORY_SURGERY
`POST /history/body/purge` records destructive document-history surgery durably without retaining the removed historical body content in the audit record.

@spec PROJECTOR.SERVER.HISTORY.ENFORCES_COMPACTION_POLICY
@planned
Server history storage can enforce the effective path-scoped history compaction policy by retaining dense recent body history and sparser older checkpoints without affecting live current document body state.
*/
