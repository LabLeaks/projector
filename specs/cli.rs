/**
@group PROJECTOR.CLI.HELP
Top-level usage surface for the projector CLI.

@spec PROJECTOR.CLI.HELP.RENDERS_TOP_LEVEL_USAGE
`projector help`, `projector --help`, and `projector -h` render the top-level usage surface without requiring a repo or server.

@spec PROJECTOR.CLI.HELP.RENDERS_COMMAND_USAGE
`projector help <command>` and `projector <command> --help` render command-specific usage for supported commands without requiring a repo or server.

@spec PROJECTOR.CLI.HELP.REJECTS_REMOVED_SYNC_NAMESPACE
When a user runs the removed `projector sync ...` namespace, projector rejects it and points them to top-level lifecycle commands such as `projector start`, `projector stop`, and `projector status`.

@group PROJECTOR.CLI.VERSION
Top-level version-reporting surface for the projector CLI.

@spec PROJECTOR.CLI.VERSION.REPORTS_RELEASE_VERSION
`projector --version` and `projector -V` print the released projector CLI version.

@spec PROJECTOR.CLI.START.RESUMES_CURRENT_REPO
`projector start` ensures the machine-global projector daemon is running and registers the current repo when it has sync entries.

@spec PROJECTOR.CLI.STOP.PAUSES_CURRENT_REPO
`projector stop` pauses syncing for the current repo by unregistering that repo from the machine-global daemon registry without stopping the daemon for other registered repos.

@spec PROJECTOR.CLI.STOP.ALL_STOPS_MACHINE_DAEMON
`projector stop --all` stops the machine-global projector daemon process for every repo.

@spec PROJECTOR.CLI.ADD
`projector add <path>` registers one repo-local path as a whole sync-entry attachment in projector configuration and in the machine-global repo registry using the chosen connected server profile for that add action.

@spec PROJECTOR.CLI.ADD.BOOTSTRAPS_LOCAL_SYNC_ENTRY
When `projector add <path>` targets an existing local file or folder, projector bootstraps that whole local sync entry against the chosen connected server profile immediately and rematerializes the resulting authoritative state locally.

@spec PROJECTOR.CLI.ADD.STATUS_SURFACE_REFLECTS_NEW_ENTRY_WITHOUT_RESTART
After `projector add <path>` succeeds, daemon-backed projector status surfaces reflect the new sync entry without requiring a manual daemon restart.

@spec PROJECTOR.CLI.ADD.REJECTS_VERSION_CONTROLLED_PATH_WITHOUT_FORCE
If `projector add <path>` targets a file or folder already tracked by the repo's VCS, projector warns and requires `--force` before adding that path to projector sync.

@spec PROJECTOR.CLI.ADD.REQUIRES_CONNECTED_SERVER_PROFILE
If no server profiles are connected, `projector add <path>` rejects the add and tells the user to run `projector connect`.

@spec PROJECTOR.CLI.REMOVE
`projector remove <path>` removes a synced repo-local path from projector configuration and unregisters it from the machine-global repo registry when no sync entries remain. `projector rm <path>` is a built-in alias for the same operation.

@spec PROJECTOR.CLI.GET.BY_ID
`projector get <sync-entry-id> [local-path]` attaches the selected whole remote sync entry by stable server-side id and materializes it locally at the requested repo-local path.

@spec PROJECTOR.CLI.GET.REJECTS_LIKELY_REPO_PATH_INPUT
When `projector get <candidate>` receives a likely repo-relative path instead of a sync-entry id, projector rejects it with guidance that `projector get` expects a sync-entry id and points the user toward remote sync-entry discovery.

@spec PROJECTOR.CLI.GET.BROWSER
Running `projector get` without an id opens a terminal browser for a chosen connected server profile's available remote sync entries, showing entry id, source repo metadata, and content preview before materialization.

@spec PROJECTOR.CLI.GET.NONINTERACTIVE_DISCOVERY_FILTERS_REMOTE_ENTRIES
Projector exposes a non-interactive remote sync-entry discovery surface that can filter available remote entries by at least source repo metadata and current remote path before local materialization.

@spec PROJECTOR.CLI.STATUS.REPORTS_CONFLICTED_TEXT_DOCUMENTS
When materialized text files contain projector conflict markers, `projector status` reports the conflicted file count and repo-relative conflicted paths.

@spec PROJECTOR.CLI.STATUS.REPORTS_SYNC_REGISTRATION
`projector status` reports whether the current repo is registered for background syncing with the machine-global daemon.

@spec PROJECTOR.CLI.STATUS.FOLLOWS_MOVED_REPO_REGISTRATION
When a synced repo directory moves on disk with its `.projector/` metadata intact, `projector status` from the new path updates the machine-global repo registry to the new repo root instead of requiring a manual stop/start dance.

@spec PROJECTOR.CLI.LOG.RENDERS_LOCAL_EVENTS
`projector log` renders the local projector event log when local bootstrap events exist.

@spec PROJECTOR.CLI.LOG.SUMMARY
`projector log` shows recent durable workspace events with path and summary, including concurrent text-merge summaries from server provenance.

@spec PROJECTOR.CLI.LOG.DISTINGUISHES_LOCAL_TRANSPORT_RESTRICTIONS
When `projector log` fails because the local runtime blocks its daemon or server transport path, projector reports that as a local runtime access failure instead of implying daemon or server sync failure.

@spec PROJECTOR.CLI.HISTORY.RENDERS_DOCUMENT_REVISIONS
`projector history <repo-relative-path>` resolves the live bound document at that path and renders recent body and path revisions from the server.

@spec PROJECTOR.CLI.HISTORY.RENDERS_WORKSPACE_RECONSTRUCTION
`projector history --cursor <workspace-cursor>` renders the reconstructed workspace manifest and live text bodies for that earlier workspace cursor.

@spec PROJECTOR.CLI.REDACT.PREVIEWS_AND_APPLIES_EXACT_TEXT_REWRITE
`projector redact <exact-text> <repo-relative-path>` previews how many retained revisions for the bound live document contain that exact text, and adding `--confirm` rewrites those retained revisions by replacing exact matches with `[REDACTED]`.

@spec PROJECTOR.CLI.REDACT.INTERACTIVE_CONFIRMATION
When `projector redact <exact-text> <repo-relative-path>` runs in an interactive terminal without `--confirm`, projector previews the matching retained revisions and can apply the redaction after terminal confirmation.

@spec PROJECTOR.CLI.REDACT.BROWSES_MATCHING_REVISIONS
When `projector redact <exact-text> <repo-relative-path>` runs in an interactive terminal without `--confirm`, projector opens a terminal browser over the matching retained revisions before applying.

@spec PROJECTOR.CLI.PURGE.PREVIEWS_AND_APPLIES_RETAINED_HISTORY_SURGERY
`projector purge <repo-relative-path>` previews how many retained revisions for the bound live document would be cleared, and adding `--confirm` clears the retained body content.

@spec PROJECTOR.CLI.PURGE.INTERACTIVE_CONFIRMATION
When `projector purge <repo-relative-path>` runs in an interactive terminal without `--confirm`, projector previews the retained revisions that would be cleared and can apply the purge after terminal confirmation.

@spec PROJECTOR.CLI.PURGE.BROWSES_CLEARABLE_REVISIONS
When `projector purge <repo-relative-path>` runs in an interactive terminal without `--confirm`, projector opens a terminal browser over the retained revisions whose body content would be cleared before applying.

@spec PROJECTOR.CLI.COMPACT.SETS_PATH_POLICY
`projector compact <repo-relative-path> --revisions <count> --frequency <count>` sets a retained-history compaction policy override for that synced path instead of compacting history immediately.

@spec PROJECTOR.CLI.COMPACT.REPORTS_EFFECTIVE_POLICY
`projector compact <repo-relative-path>` without mutation flags reports the effective retained-history compaction policy for that path together with whether it comes from a file override, an ancestor-folder override, or the inherited default.

@spec PROJECTOR.CLI.COMPACT.INHERITS_PATH_POLICY
`projector compact <repo-relative-path> --inherit` removes that path's retained-history compaction policy override so the path falls back to the nearest inherited policy.

@spec PROJECTOR.CLI.CONNECT.PERSISTS_GLOBAL_PROFILE_REGISTRY
`projector connect` interactively, or `projector connect --id <profile> --server <server-addr>`, persists one connected server profile in machine-global projector state.

@spec PROJECTOR.CLI.CONNECT.REPORTS_SERVER_STATUS
`projector connect status` reports all connected server profiles with ids, reachability, usage counts, and the repo-local sync-entry paths currently attached through each profile.

@spec PROJECTOR.CLI.CONNECT.STATUS_CLARIFIES_LOCAL_ATTACHMENT_SCOPE
`projector connect status` explicitly labels its repo-local path view as local attachment state rather than authoritative remote sync-entry inventory.

@spec PROJECTOR.CLI.DISCONNECT.REMOVES_CONNECTED_PROFILE
`projector disconnect <profile>` warns with the repo-local paths that will become desynced and then removes that machine-global connected server profile when confirmed.

@spec PROJECTOR.CLI.DEPLOY.GUIDED_REMOTE_SETUP
`projector deploy` uses an interactive flow to configure and provision a remote self-host target, defaulting to a sysbox-isolated container that runs one projector-server binary against one SQLite database file.

@spec PROJECTOR.CLI.DEPLOY.USES_SYSBOX_ISOLATION
`projector deploy` provisions the default BYO server inside a sysbox-backed container rather than launching projector-server directly as an unmanaged host process.

@spec PROJECTOR.CLI.DEPLOY.REGISTERS_SERVER_PROFILE
After provisioning a remote self-host target, `projector deploy` registers the resulting server as a named global server profile ready for `projector connect` and `projector add`.

@spec PROJECTOR.CLI.DOCTOR.REPORTS_PROFILE_AND_REACHABILITY
`projector doctor` reports how many machine-global server profiles are connected, whether repo-local sync entries refer to registered server profiles, and whether each registered profile referenced by the repo is reachable.

@spec PROJECTOR.CLI.DOCTOR.REPORTS_SYNC_ENTRY_SANITY
`projector doctor` reports one line of sanity information for each repo-local sync entry, including local path, kind, server profile, gitignore state, tracked-by-git state, and local path existence.

@spec PROJECTOR.CLI.DOCTOR.REPORTS_RUNTIME_AND_SYNC_ISSUES
`projector doctor` reports machine-daemon state, repo registration state, runtime lease state, and the repo's recent sync issue count, then summarizes the result as `doctor_status: ok|warn|error`.
*/
