//! Guards the facts that must agree before a version is published.
//!
//! kasl-server ships to two places - GitHub and crates.io - and each renders
//! its own copy of the README. Drift is only visible after publishing, when
//! it is too late to take back, so these checks run in CI instead.

use std::fs;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: impl AsRef<Path>) -> String {
    let path = repo_root().join(path);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// Reads a top-level `key = "value"` from the `[package]` block of Cargo.toml.
///
/// Deliberately naive: it stops at the next section, which is all these checks
/// need, and avoids a TOML parser as a dev-dependency.
fn cargo_field(key: &str) -> String {
    let manifest = read("Cargo.toml");
    for line in manifest.lines() {
        let line = line.trim();
        // `version` also appears under [dependencies] and in every dependency.
        if line.starts_with('[') && line != "[package]" {
            break;
        }
        let Some((name, value)) = line.split_once('=') else { continue };
        // Exact match, so `rust-version` cannot answer a lookup for `version`.
        if name.trim() != key {
            continue;
        }
        return value.trim().trim_matches('"').to_string();
    }
    panic!("`{key}` not found in the [package] block of Cargo.toml");
}

#[test]
fn readme_links_resolve_off_github() {
    // The same file is rendered on crates.io, where a relative path has no
    // repository to resolve against: the banner turns into a broken image
    // and the links 404.
    let readme = read("README.md");

    for (line_no, line) in readme.lines().enumerate() {
        for (marker, kind) in [("src=\"", "image"), ("](", "link")] {
            let mut rest = line;
            while let Some(at) = rest.find(marker) {
                let target = &rest[at + marker.len()..];
                let end = if marker == "](" { ')' } else { '"' };
                let target = &target[..target.find(end).unwrap_or(target.len())];

                let relative = !target.starts_with("http") && !target.starts_with('#') && !target.is_empty();
                assert!(
                    !relative,
                    "README line {}: relative {kind} `{target}` breaks on crates.io; use an absolute URL",
                    line_no + 1
                );

                rest = &rest[at + marker.len()..];
            }
        }
    }
}

#[test]
fn the_changelog_covers_the_version_being_shipped() {
    // The tag drives an irreversible publish, and the release notes are cut
    // from the changelog. A manifest bumped without a changelog entry ships a
    // version nobody can read the changes of.
    let version = cargo_field("version");
    let changelog = read("CHANGELOG.md");
    let heading = format!("## [{version}]");

    assert!(
        changelog.contains(&heading),
        "CHANGELOG.md has no `{heading}` section; run `git-cliff --tag v{version}` before tagging"
    );
}

#[test]
fn the_readme_shows_the_version_being_shipped() {
    // The README carries a transcript of a live run. When the manifest moves
    // and the transcript does not, the storefront advertises the previous
    // release's output as if it were this one.
    let version = cargo_field("version");
    let readme = read("README.md");

    assert!(
        readme.contains(&format!("\"version\":\"{version}\"")),
        "README.md does not show version {version} in its transcript; re-run the server and paste the current output"
    );
}

#[test]
fn readme_is_not_duplicated() {
    // One README for every storefront. A second copy is where descriptions
    // start to drift; the frontend and docs milestones must reuse the root
    // file rather than fork it.
    for candidate in ["docs/README.md", "frontend/README.md"] {
        let duplicate = repo_root().join(candidate);
        assert!(
            !duplicate.exists(),
            "{candidate} exists; it will drift from the root README, which is the single source"
        );
    }
}
