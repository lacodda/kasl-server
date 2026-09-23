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

#[test]
fn the_package_carries_the_built_web_ui() {
    // The UI is embedded in the binary (ADR 0012), and `frontend/dist` is
    // gitignored - so only `include` puts it in the crate. Drop that line and
    // `cargo install kasl-server` stops compiling for everyone, which is
    // discovered by a stranger rather than by us.
    let manifest = read("Cargo.toml");
    assert!(
        manifest.contains("\"frontend/dist/**\""),
        "Cargo.toml must include frontend/dist in the package, or `cargo install` cannot build it"
    );
}

#[test]
fn the_web_ui_was_built_before_this_release() {
    // A placeholder `dist` compiles fine and produces a binary that answers
    // "no web UI was built into this binary" - a release that looks complete
    // and serves nothing. The check is for the built document, not the
    // directory, because the directory always exists.
    let index = repo_root().join("frontend/dist/index.html");
    assert!(
        index.exists(),
        "frontend/dist/index.html is missing; run `pnpm --dir frontend build` before packaging or tagging"
    );
}

#[test]
fn the_install_compose_names_the_image_this_repository_publishes() {
    // The install path is "download this file, run it". If the image name in
    // it drifts from what `image.yml` pushes, that instruction fails for a
    // stranger and works for us - we have the source and never pull.
    let compose = read("docker-compose.install.yml");
    let workflow = read(".github/workflows/image.yml");

    assert!(
        compose.contains("ghcr.io/lacodda/kasl-server:"),
        "the install compose must pull the published image"
    );
    assert!(
        workflow.contains("ghcr.io/${{ github.repository }}"),
        "the image workflow must publish under the repository's own name"
    );
    // Webhook destinations have names nobody can list in advance
    // (`KASL_WEBHOOK_<NAME>`), so an `environment:` block alone would drop
    // every one of them on the way into the container - the guide's `.env`
    // would configure a server that never hears it.
    for file in ["docker-compose.install.yml", "docker-compose.prod.yml"] {
        let text = read(file);
        assert!(
            text.contains("env_file:") && text.contains("path: .env"),
            "{file} must pass .env through to the server"
        );
    }
    assert!(
        !compose.contains("build:"),
        "the install compose must not build from source; that is docker-compose.prod.yml"
    );
}

#[test]
fn the_docs_document_every_environment_variable() {
    // The configuration table is the only place an operator learns these
    // exist. A variable added to the code and not to the table is invisible
    // until someone reads the source, which is not what a self-hosted product
    // can ask of them.
    //
    // The table used to live in the README and moved to the documentation site
    // when the README became a shopfront - and this check stayed pointed at
    // the README, where it then passed by finding nothing to compare against
    // on the way to failing on the first variable. A gate follows the text it
    // guards.
    // The destinations are read by prefix in their own module rather than by
    // name in `config.rs`, and a scan of one file would never see the prefix.
    // Only the module's code, not its tests: those name example destinations
    // (`KASL_WEBHOOK_TEAM_CHAT`) that no operator is meant to set.
    let destinations = read("src/webhooks/destination.rs");
    let destinations = destinations.split("#[cfg(test)]").next().unwrap_or_default().to_string();
    assert!(
        destinations.contains("\"KASL_WEBHOOK_\""),
        "the webhook prefix moved; point this scan at where it lives now"
    );
    let config = read("src/config.rs") + &destinations;
    let reference = read("docs/src/content/docs/reference/configuration.md");

    // Every `"KASL_*"` literal, not only the ones passed straight to `lookup`:
    // the limits and the flags go through `positive(...)` and `boolean(...)`,
    // and a scan anchored on `lookup(` found four of the eight - and would
    // have gone on saying nothing about a ninth.
    let mut checked = Vec::new();
    let mut rest = config.as_str();
    while let Some(at) = rest.find("\"KASL_") {
        let name = &rest[at + 1..];
        let Some(end) = name.find('"') else { break };
        let variable = &name[..end];
        rest = &name[end..];

        // A variable's name, not an error message that happens to start with
        // one: `"KASL_SERVER_ADDR is not a valid socket address: {addr}"` is a
        // string in this file too.
        if !variable.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') {
            continue;
        }

        assert!(
            reference.contains(variable),
            "{variable} is read by the server but missing from the configuration reference"
        );
        if !checked.contains(&variable) {
            checked.push(variable);
        }
    }

    // A scan that finds nothing asserts nothing. Without this the test stays
    // green after a rename that takes every variable out of its reach - which
    // is exactly how its predecessor came to be checking the wrong file.
    assert!(
        checked.len() >= 8,
        "only {} variables were found in config.rs ({checked:?}); the scan is looking in the wrong place",
        checked.len()
    );
}
