/**
@module PROJECTOR.EDGE.OBSERVABILITY_CLI
Coordinates the day-to-day operational diagnostics surfaces by delegating status, doctor, log, and conflict scanning to narrower edge modules.
*/
// @fileimplements PROJECTOR.EDGE.OBSERVABILITY_CLI
mod conflicts;
mod doctor;
mod log;
mod status;

pub(crate) use doctor::run_doctor;
pub(crate) use log::run_log;
pub(crate) use status::run_status;
