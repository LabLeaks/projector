/**
@spec PROJECTOR.CLI.SYNC.MANAGES_MACHINE_DAEMON_PROCESS
`projector sync start`, `projector sync status`, and `projector sync stop` manage and report the machine-global projector daemon process through machine-global projector state instead of doubling as repo-local bind commands.

@spec PROJECTOR.CLI.SYNC.SCOPES_LOCAL_WORK_TO_CHANGED_ENTRIES
The machine-global sync daemon scopes local sync work to sync entries whose watched paths changed locally instead of rerunning every registered sync entry on every daemon tick.

@spec PROJECTOR.CLI.SYNC.USES_SLOW_BACKSTOPS
The machine-global sync daemon uses slower registry refresh, filesystem polling backstop, and idle remote sweep intervals than its main wakeup tick so idle syncing does not tight-loop full-world work.

@spec PROJECTOR.CLI.SYNC.REUSES_SERVER_TRANSPORTS
During one machine-daemon run, projector reuses HTTP transport sessions across sync entries that target the same server address instead of constructing isolated transports for every entry pass.

@spec PROJECTOR.CLI.SYNC.COALESCES_LOCAL_EVENT_BURSTS
The machine-global sync daemon coalesces bursts of local watcher events for the same repo into one targeted sync batch after a short debounce instead of running near-duplicate sync passes for every event tick.

@spec PROJECTOR.CLI.ADD
`projector add <path>` registers one repo-local path as a whole sync-entry attachment in projector configuration and in the machine-global repo registry using the chosen connected server profile for that add action.

@spec PROJECTOR.CLI.ADD.BOOTSTRAPS_LOCAL_SYNC_ENTRY
When `projector add <path>` targets an existing local file or folder, projector bootstraps that whole local sync entry against the chosen connected server profile immediately and rematerializes the resulting authoritative state locally.

@spec PROJECTOR.CLI.ADD.REJECTS_VERSION_CONTROLLED_PATH_WITHOUT_FORCE
If `projector add <path>` targets a file or folder already tracked by the repo's VCS, projector warns and requires `--force` before adding that path to projector sync.

@spec PROJECTOR.CLI.ADD.REQUIRES_CONNECTED_SERVER_PROFILE
If no server profiles are connected, `projector add <path>` rejects the add and tells the user to run `projector connect`.

@spec PROJECTOR.CLI.REMOVE
`projector remove <path>` removes a synced repo-local path from projector configuration and unregisters it from the machine-global repo registry when no sync entries remain. `projector rm <path>` is a built-in alias for the same operation.

@spec PROJECTOR.CLI.GET.BY_ID
`projector get <sync-entry-id> [local-path]` attaches the selected whole remote sync entry by stable server-side id and materializes it locally at the requested repo-local path.

@spec PROJECTOR.CLI.GET.BROWSER
Running `projector get` without an id opens a terminal browser for a chosen connected server profile's available remote sync entries, showing entry id, source repo metadata, and content preview before materialization.

@spec PROJECTOR.CLI.STATUS.REPORTS_CONFLICTED_TEXT_DOCUMENTS
When materialized text files contain projector conflict markers from a concurrent merge, `projector status` reports the conflicted file count and repo-relative conflicted paths.

@spec PROJECTOR.CLI.LOG.RENDERS_LOCAL_EVENTS
`projector log` renders the local projector event log when local bootstrap events exist.

@spec PROJECTOR.CLI.HISTORY.RENDERS_DOCUMENT_REVISIONS
`projector history <repo-relative-path>` resolves the live bound document at that path and renders recent body and path revisions from the server.

@spec PROJECTOR.CLI.HISTORY.RENDERS_WORKSPACE_RECONSTRUCTION
`projector history --cursor <workspace-cursor>` renders the reconstructed workspace manifest and live text bodies for that earlier workspace cursor.

@spec PROJECTOR.BINDING.REPO_LOCAL_METADATA
Projector keeps checkout-local sync configuration and runtime metadata under `.projector/`, outside the configured projection mounts.

@spec PROJECTOR.BINDING.SERVER_PROFILE
Each repo-local sync entry refers to a named global server profile rather than treating a raw server address as the primary long-term binding contract.

@spec PROJECTOR.BINDING.ONE_SERVER_PROFILE_PER_ENTRY
Each path-scoped sync entry refers to exactly one authoritative server profile at a time even though the machine may know about multiple server profiles globally.

@spec PROJECTOR.BINDING.PATH_SCOPED_ENTRIES
Repo-local projector configuration stores one or more path-scoped sync entries rather than treating the entire repo as one indivisible remote binding.

@spec PROJECTOR.BINDING.WHOLE_REMOTE_ENTRY
Each repo-local sync attachment refers to one whole remote sync entry by stable server-side sync-entry id; projector does not attach only a subset of an existing remote sync entry.

@spec PROJECTOR.WORKSPACE.PROJECTION_ROOT
Projector materializes synced private context under one or more configured repo-local gitignored projection mounts rather than a hardcoded repo root.

@spec PROJECTOR.WORKSPACE.TEXT_ONLY
v0 materializes only UTF-8 text files and directories under the configured projection mounts.

@spec PROJECTOR.SYNC.TEXT_CONVERGENCE
UTF-8 text files under the configured projection mounts converge across synced checkouts for the same workspace through deterministic server-side three-way merge, preserving both sides with conflict markers when concurrent edits overlap.

@spec PROJECTOR.SYNC.FILE_LIFECYCLE
The synced workspace reconciles UTF-8 text file creation, body edits, and deletion through a server-backed manifest rather than by treating local disk as the source of truth.

@spec PROJECTOR.PROVENANCE.EVENT_LOG
projector records durable provenance for workspace file lifecycle and body updates with path, timestamp, and operation summary.

@spec PROJECTOR.DISTRIBUTION.RELEASE_FLOW.CHECKLIST
Before publishing, projector's local release script interactively confirms easy-to-forget release tasks such as updating public docs, updating `CHANGELOG.md`, bumping workspace package versions, and running core validation.

@spec PROJECTOR.DISTRIBUTION.RELEASE_FLOW.DRY_RUN
Projector's local release script dry-run prints the prerelease checklist and publication commands without moving the `main` bookmark, creating a tag, pushing to origin, or updating Homebrew.

@spec PROJECTOR.DISTRIBUTION.RELEASE_FLOW.MATCHES_WORKSPACE_VERSION
Projector's local release script requires the requested release tag to exactly match the shared workspace package version across projector crates.

@spec PROJECTOR.DISTRIBUTION.RELEASE_FLOW.PUSHES_MAIN_AND_TAG
Projector's local release script publishes the chosen revision by pushing the `main` bookmark with Jujutsu and the release Git tag to origin.

@spec PROJECTOR.DISTRIBUTION.RELEASE_FLOW.VERIFIES_GITHUB_RELEASE
After pushing the release tag, projector's local release script waits for the GitHub Release artifacts to publish and verifies the expected asset set.

@spec PROJECTOR.DISTRIBUTION.RELEASE_FLOW.UPDATES_HOMEBREW
After the GitHub Release is published, projector's local release script updates the Homebrew tap formula for `projector` and verifies that formula against the published projector CLI release archives.

@spec PROJECTOR.DISTRIBUTION.GITHUB_RELEASES.WORKFLOW
Projector keeps a GitHub Actions release workflow in `.github/workflows/release.yml`.

@spec PROJECTOR.DISTRIBUTION.GITHUB_RELEASES.ASSET_MANIFEST
Projector keeps an in-repo release asset manifest that names the GitHub release assets and Homebrew-relevant projector CLI archives expected by its release verification tooling.

@spec PROJECTOR.DISTRIBUTION.HOMEBREW.FORMULA_AUTOMATION
Projector ships Homebrew tap update and verification automation for `Formula/projector.rb` in `LabLeaks/homebrew-tap`, scoped to the user-facing `projector` CLI binary rather than the remotely deployed projector-server binary.

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

@spec PROJECTOR.HISTORY.SURGICAL_REDACTION
@planned
Projector can surgically redact or purge a selected sensitive document body revision by sequence without requiring full document or workspace history deletion.

@spec PROJECTOR.HISTORY.RETENTION_POLICY
@planned
Projector can attach a per-sync-entry history retention policy that controls revision retention and compression independently for each synced object.

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

@spec PROJECTOR.SERVER.HISTORY.REDACTS_DOCUMENT_BODY_REVISION
@planned
`POST /history/body/redact` can surgically redact or purge a selected sensitive document body revision and records that destructive history surgery durably.

@spec PROJECTOR.SERVER.HISTORY.ENFORCES_RETENTION_POLICY
@planned
Server history storage can enforce per-sync-entry revision retention and compression policy without affecting the live current body state for that sync entry.

@spec PROJECTOR.SERVER.AUTH
@planned
Server-side identity and authorization behavior.

@spec PROJECTOR.SERVER.AUTH.RBAC
@planned
Authenticated workspace members can be granted at least `read_only` and `read_write` roles, with write endpoints rejecting read-only actors.

@spec PROJECTOR.SERVER.HOSTING
@planned
Single-user v0 assumes one or more user-supplied server profiles rather than a blessed local embedded server mode.

@spec PROJECTOR.SERVER.HOSTING.BYO_SERVER
@planned
The base-case single-user deployment story is bring-your-own private servers registered as named profiles and reachable from the repos, machines, and agents that use them.

@spec PROJECTOR.SERVER.HOSTING.SQLITE_DEFAULT
@planned
For single-user BYO deployments, SQLite is the default server store because it keeps deploy, backup, and remote operation simple.

@spec PROJECTOR.SERVER.HOSTING.POSTGRES_ADVANCED
@planned
Postgres is the advanced server store for managed cloud, PaaS, or more operationally serious deployments rather than the default single-user BYO path.

@spec PROJECTOR.CLI.CONNECT.PERSISTS_GLOBAL_PROFILE_REGISTRY
`projector connect` interactively, or `projector connect --id <profile> --server <server-addr>`, persists one connected server profile in machine-global projector state.

@spec PROJECTOR.CLI.CONNECT.REPORTS_SERVER_STATUS
`projector connect status` reports all connected server profiles with ids, reachability, usage counts, and the repo-local sync-entry paths currently attached through each profile.

@group PROJECTOR.CLI.DISCONNECT
Machine-global server profile removal behavior.

@spec PROJECTOR.CLI.DISCONNECT.REMOVES_CONNECTED_PROFILE
`projector disconnect <profile>` warns with the repo-local paths that will become desynced and then removes that machine-global connected server profile when confirmed.

@spec PROJECTOR.CLI.DEPLOY.GUIDED_REMOTE_SETUP
`projector deploy` uses an interactive flow to configure and provision a remote self-host target, defaulting to a sysbox-isolated container that runs one projector-server binary against one SQLite database file and leaving Postgres-oriented targets to advanced flows.

@spec PROJECTOR.CLI.DEPLOY.USES_SYSBOX_ISOLATION
`projector deploy` provisions the default BYO server inside a sysbox-backed container rather than launching projector-server directly as an unmanaged host process.

@spec PROJECTOR.CLI.DEPLOY.REGISTERS_SERVER_PROFILE
After provisioning a remote self-host target, `projector deploy` registers the resulting server as a named global server profile ready for `projector connect` and `projector add`.

@spec PROJECTOR.CLOUD
@planned
Projector can later offer a managed cloud path that removes self-host deployment entirely and adds team management, sharing, and authorization features.

@group PROJECTOR.CLI.DOCTOR
Explicit setup and diagnostics behavior for projector.

@spec PROJECTOR.CLI.DOCTOR.REPORTS_PROFILE_AND_REACHABILITY
`projector doctor` reports how many machine-global server profiles are connected, whether repo-local sync entries refer to registered server profiles, and whether each registered profile referenced by the repo is reachable.

@spec PROJECTOR.CLI.DOCTOR.REPORTS_SYNC_ENTRY_SANITY
`projector doctor` reports one line of sanity information for each repo-local sync entry, including local path, kind, server profile, gitignore state, tracked-by-git state, and local path existence.

@spec PROJECTOR.CLI.DOCTOR.REPORTS_RUNTIME_AND_SYNC_ISSUES
`projector doctor` reports machine-daemon state, repo registration state, runtime lease state, and the repo's recent sync issue count, then summarizes the result as `doctor_status: ok|warn|error`.

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

@spec PROJECTOR.CLI.LOG.SUMMARY
`projector log` shows recent durable workspace events with path and summary, including conflicting merge summaries from server provenance.
*/
