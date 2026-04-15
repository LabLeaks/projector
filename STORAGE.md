# Server Storage

`projector` should use SQLite as the default single-user BYO server store for v0.

Postgres should remain available, but as the advanced backend for cloud, PaaS, or more operationally serious deployments.

## Storage stance

The point of the default backend is not prestige. It is deploy and operator friction.

For the normal one-person BYO path, SQLite is the right trade:

- one file
- easy remote deploy
- easy backup and copy
- transactional enough for v0 manifest, history, restore, and sync-entry discovery
- no external database service to provision or babysit

For the advanced path, Postgres is still useful:

- stronger multi-writer operational story
- better fit for managed cloud and PaaS hosting
- better long-term shape for richer multi-user features

So the product/storage split should be:

- SQLite by default for single-user self-host
- Postgres for advanced or managed environments
- no bespoke CRDT-native or event-store database in v0

## Default SQLite shape

The default SQLite backend can stay intentionally simple.

It should provide:

- transactional bootstrap writes
- transactional manifest writes
- append-only provenance
- append-only body and path revision history
- enough queryability to support restore and remote sync-entry discovery

For v0, that does not require a highly normalized schema. A pragmatic SQLite layout with transactional workspace snapshots plus append-only event and revision rows is acceptable if it keeps the deploy story boring.

## Postgres shape

Postgres is still the right place to grow into a more normalized backend.

When the deployment shape actually needs it, the server can use relational tables such as:

- `workspaces`
- `workspace_mounts`
- `documents`
- `document_paths`
- `document_body_revisions`
- `document_body_updates`
- `document_body_snapshots`
- `provenance_events`

That remains the better fit for cloud, PaaS, or future multi-user work. It just should not be the forced default for one-person BYO hosting.

## Bootstrap model

Regardless of backend, bootstrap should stay snapshot-first.

Preferred flow:

1. read live manifest state
2. read current body state for live documents
3. return one complete snapshot to the client

Steady-state sync can stay incremental, but reconnect should always be able to fall back to a clean snapshot.

## Write model

Regardless of backend, manifest and provenance should be written transactionally.

Typical file lifecycle mutation:

1. validate workspace identity and mount ownership
2. update current manifest and body state
3. append provenance
4. append body and path history as needed
5. commit

Do not treat the local checkout as the authority for identity or path lifecycle.

## Why SQLite first

`projector` v0 is trying to make self-host feel like a tool, not a stack.

That means the default deployment should be closer to:

- copy one binary
- point it at one SQLite file
- run it under systemd, launchd, Docker, or equivalent

and farther from:

- provision and operate a separate database service before sync is even useful

That is why SQLite should be the default, even though Postgres remains important for later lanes.
