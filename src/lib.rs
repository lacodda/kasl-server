//! kasl-server as a library.
//!
//! The binary is a thin wrapper: configuration, a pool, migrations, then
//! `app::router`. Everything else lives here so integration tests can drive
//! the real router and the real provisioning path instead of a copy that
//! drifts from what ships.

pub mod admin;
pub mod app;
pub mod audit;
pub mod auth;
pub mod backup;
pub mod config;
pub mod demo;
pub mod department;
pub mod error;
pub mod heartbeat;
pub mod heatmap;
pub mod import;
pub mod ingest;
pub mod login;
pub mod me;
pub mod model;
pub mod privacy;
pub mod provision;
pub mod session;
pub mod team;
pub mod web;

/// The embedded migrations, applied on startup and by the tests.
///
/// Declared here rather than at each call site so there is one answer to "what
/// schema does this build carry" - the binary and the test suite cannot end up
/// migrating differently.
pub fn migrator() -> sqlx::migrate::Migrator {
    sqlx::migrate!()
}
