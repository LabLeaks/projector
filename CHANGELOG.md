# Changelog

## 0.2.2

`0.2.2` is a dogfood hardening release for sync-entry recovery, diagnostics, and local filesystem move handling.

### Highlights

- Added non-interactive remote sync-entry discovery with `projector get --list`, plus source-repo and remote-path filters.
- Improved `projector get` guidance when users pass a workspace id or likely repo-relative path instead of a sync-entry id.
- Hardened `projector add` so failed repo registration or bootstrap attempts roll back the repo-local sync-entry config.
- Made `projector log` distinguish local runtime or sandbox transport restrictions from proven daemon or server sync failures.
- Clarified that `projector connect status` reports local attachment state, not authoritative remote inventory.
- Preserved document ids when watcher events report a renamed folder under a synced directory mount instead of individual child-file moves.
- Relocated moved or renamed directory sync-entry roots by updating the repo-local binding to the unique matching moved root instead of treating the old root disappearance as remote deletion.

## 0.2.1

`0.2.1` is a small patch release on top of the new `0.2.x` baseline.

### Highlights

- Added explicit top-level help handling for `projector help`, `projector --help`, and `projector -h`.
- Added explicit version reporting for `projector --version` and `projector -V`.
- Kept the release/install smoke surface honest after the first public `0.2.0` cut.

## 0.2.0

`0.2.0` is the first real public release after initial dogfood. It turns `projector` from durable private file sync into a serious private shared-context system for repo-local text.

### Highlights

- Replaced the old server-side three-way-merge text convergence path with CRDT-backed canonical text state for UTF-8 text documents.
- Promoted retained history into a first-class product surface with readable snapshot-and-diff history, restore, redact, and purge.
- Added explicit history-surgery CLI flows:
  - `projector redact`
  - `projector purge`
  - `projector compact`
- Added path-scoped retained-history compaction policy with server-side enforcement and nearest-ancestor inheritance.
- Hardened release quality gates with a local Codex release-review wrapper, stronger release-pipeline verification, and tighter spec/proof organization.

### User-facing notes

- This release is still single-user and BYO-server first.
- The normal deployment path remains `projector deploy` to a sysbox-isolated remote server with SQLite as the default store.
- `projector` now treats retained history as an explicit feature rather than an internal side effect, so history inspection and cleanup are safer and more legible.

### Internal step change

- The body/history model changed substantially in `0.2.0`: canonical live text state is now CRDT-backed, and retained history is stored as checkpoint/update history rendered back as readable snapshot-and-diff output.
- This is the new baseline for real `projector` dogfooding. Cloud/RBAC and broader team features are still explicitly later work.

## 0.1.0

- Initial single-user `projector` release.
- Added BYO remote server support with SQLite as the default store and sysbox-isolated deploy.
- Added machine-global sync daemon lifecycle, server profile management, and path-scoped sync entries.
- Added remote-first `get`, local-first `add`, `doctor`, history, and restore flows.
