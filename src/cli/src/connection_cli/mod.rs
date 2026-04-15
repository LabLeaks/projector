/**
@module PROJECTOR.EDGE.CONNECTION_CLI
Owns the thin command seam for server connection management, disconnection warnings, and remote BYO deployment.
*/
// @fileimplements PROJECTOR.EDGE.CONNECTION_CLI
mod args;
mod deploy;
mod profiles;
mod prompts;

pub(crate) use deploy::run_deploy;
pub(crate) use profiles::{
    resolve_profile_for_action, run_connect, run_disconnect, server_addr_reachable,
};
