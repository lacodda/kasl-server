//! Rebuilds the crate when a migration is added, and gives the Windows
//! executable its icon.
//!
//! `sqlx::migrate!()` embeds `migrations/` at compile time, and Cargo only
//! reruns a macro when a file it knows about changes. A new migration is a new
//! file, not a changed one, so without this the binary and the test suite keep
//! migrating to the schema of the last build - a test of the new migration
//! then runs against the old list and fails for a reason that is not in the
//! code. This is the build script sqlx itself recommends.
//!
//! The release ships a Windows build, and a console program with no icon
//! resource is drawn with Windows' generic executable icon in Explorer, on a
//! pinned shortcut and in its properties. `assets/icon.ico` is the line's mark
//! cut level by level (docs/export-assets.mjs); it sits inside the package, so
//! `cargo install` on Windows embeds it too. The embedding is unconditional
//! there: a package that lost the file fails to build instead of quietly
//! shipping without a mark.
fn main() {
    println!("cargo:rerun-if-changed=migrations");

    // The host decides whether winresource is available at all; the target
    // decides whether an icon belongs in the output.
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=assets/icon.ico");
        winresource::WindowsResource::new()
            .set_icon("assets/icon.ico")
            .compile()
            .expect("failed to embed the Windows icon");
    }
}
