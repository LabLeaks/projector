/**
@module PROJECTOR.SERVER.HTTP
Coordinates the projector HTTP edge by composing listener lifecycle and route handlers over the server storage boundary.
*/
// @fileimplements PROJECTOR.SERVER.HTTP
mod handlers;
mod runtime;

pub use runtime::{serve, serve_file_backed, serve_postgres, serve_sqlite, spawn_background};
