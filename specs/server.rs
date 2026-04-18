/**
@spec PROJECTOR.SERVER.AUTH.RBAC
@planned
Authenticated workspace members can be granted at least `read_only` and `read_write` roles, with write endpoints rejecting read-only actors.

@spec PROJECTOR.SERVER.HOSTING.BYO_SERVER
Single-user projector workflows attach repos through one or more named user-supplied server profiles rather than through a required embedded local server mode.

@spec PROJECTOR.SERVER.HOSTING.SQLITE_DEFAULT
For the single-user BYO deploy path, `projector deploy` defaults the remote server to one projector-server binary backed by one SQLite database file.

@spec PROJECTOR.SERVER.HOSTING.POSTGRES_ADVANCED
Alongside the default SQLite BYO deploy path, `projector-server serve --postgres-url <url>` supports Postgres as an available backend for more operationally serious deployments.

@spec PROJECTOR.SERVER.DOCUMENTS.CREATE_TRANSACTIONAL_DOCUMENT
`POST /documents/create` creates a text document in the bound workspace, persists its manifest and body state, records provenance, and makes it visible in the next bootstrap snapshot.

@spec PROJECTOR.SERVER.DOCUMENTS.UPDATE_TRANSACTIONAL_DOCUMENT
The server-side document update path updates a text document in the bound workspace, persists the new body state, records provenance, and makes the edited body visible in the next bootstrap snapshot.

@spec PROJECTOR.SERVER.DOCUMENTS.DELETE_TRANSACTIONAL_DOCUMENT
The server-side document delete path deletes a text document in the bound workspace, persists the deleted manifest state, records provenance, and makes the deletion visible in the next bootstrap snapshot.

@spec PROJECTOR.SERVER.DOCUMENTS.MOVE_TRANSACTIONAL_DOCUMENT
The server-side document move path updates a text document path in the bound workspace, persists the manifest path change, records provenance, and preserves the document id in the next bootstrap snapshot.

@spec PROJECTOR.SERVER.DOCUMENTS.REJECTS_STALE_MANIFEST_WRITES
Manifest-changing document writes reject stale workspace cursors rather than silently applying against newer server state.

@spec PROJECTOR.SERVER.SYNC.CHANGES_SINCE_RETURNS_CHANGED_DOCUMENTS
`POST /changes/since` returns only the documents changed after the requested workspace cursor, together with the next cursor for steady-state sync.

@spec PROJECTOR.SERVER.SYNC_ENTRIES.LIST
The server can list available remote sync entries on a requested server profile with stable id, current remote path, kind, source repo metadata, and lightweight preview metadata.

@spec PROJECTOR.CLOUD
@planned
Projector can later offer a managed cloud path that removes self-host deployment entirely and adds team management, sharing, and authorization features.
*/
