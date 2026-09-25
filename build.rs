//! Rebuilds the crate when a migration is added.
//!
//! `sqlx::migrate!()` embeds `migrations/` at compile time, and Cargo only
//! reruns a macro when a file it knows about changes. A new migration is a new
//! file, not a changed one, so without this the binary and the test suite keep
//! migrating to the schema of the last build - a test of the new migration
//! then runs against the old list and fails for a reason that is not in the
//! code. This is the build script sqlx itself recommends.
fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
