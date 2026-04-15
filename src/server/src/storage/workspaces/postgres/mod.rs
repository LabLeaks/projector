/**
@module PROJECTOR.SERVER.POSTGRES_WORKSPACES
Coordinates Postgres-backed workspace bootstrap, sync-entry discovery, and delta reads through narrower Postgres workspace modules.
*/
// @fileimplements PROJECTOR.SERVER.POSTGRES_WORKSPACES
mod bootstrap;
mod discovery;

pub(crate) use bootstrap::{postgres_bootstrap_workspace, postgres_changes_since};
pub(crate) use discovery::postgres_list_sync_entries;
