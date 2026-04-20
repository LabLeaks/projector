# Changelog

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
