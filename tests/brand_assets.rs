//! The mark, held against the level rule of the line and against its copies.
//!
//! kasl-server's mark has three masters - a filled gold tile (S), the plated
//! tile with the code alone (M), and the plated tile with the chevrons and the
//! bracket under the code (L) - and which one a raster or a placement uses is
//! decided by how big it is, never by what it is for. Getting that wrong is
//! invisible in a diff and obvious on screen, and it has shipped in this line
//! several times: kasl's docs header, kilna's desktop icon, sefy's whole
//! `.ico` - each caught by the owner's eye, releases late.
//!
//! Nothing catches it by looking at the file. A container cut entirely from
//! one master is perfectly well-formed: valid PNGs of the right dimensions,
//! a correct directory. So this gate decodes the images and reads pixels.
//!
//! Everything here reads files in the repository. An earlier attempt in the
//! line compared the SVGs against a registry kept in a notes vault, which CI
//! does not have: the check passed by finding nothing to check.

use std::fs;
use std::path::{Path, PathBuf};

use image::RgbaImage;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: impl AsRef<Path>) -> Vec<u8> {
    let path = repo_root().join(path);
    fs::read(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

fn read_text(path: &str) -> String {
    String::from_utf8(read(path)).unwrap_or_else(|_| panic!("{path} is not UTF-8"))
}

/// The three masters. docs/export-assets.mjs rasterizes every PNG from them.
const S: &str = "assets/logo-s.svg";
const M: &str = "assets/logo-m.svg";
const L: &str = "assets/logo.svg";

/// Which master a raster was cut from, read off its pixels.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Level {
    /// The hex filled with the product colour.
    Small,
    /// The dark plate with an outline and the code, nothing under it.
    Medium,
    /// The plate with the metaphor - two chevrons entering a bracket.
    Large,
}

/// The master the line's rule asks for at this many pixels.
///
/// Written out here rather than imported: the exporter is JavaScript, and this
/// is the independent reading of the same rule.
fn level_for(size: u32) -> Level {
    match size {
        0..=27 => Level::Small,
        28..=63 => Level::Medium,
        _ => Level::Large,
    }
}

/// The pixel at a point given in the masters' 0..100 viewBox.
fn pixel(image: &RgbaImage, x: u32, y: u32) -> [u8; 4] {
    let (width, height) = image.dimensions();
    image.get_pixel(width * x / 100, height * y / 100).0
}

fn brightness(rgba: [u8; 4]) -> u32 {
    u32::from(rgba[0]) + u32::from(rgba[1]) + u32::from(rgba[2])
}

/// Below the code, on the axis: only S puts colour there.
const FILL_PROBE: (u32, u32) = (50, 85);
/// On the bracket's upright: only L draws there.
const METAPHOR_PROBE: (u32, u32) = (64, 69);

/// Decide the level of one image from two pixels.
///
/// Each sits on something only one master puts there, so together they name
/// the master. A gate that only told filled from plated would accept the M
/// tile at 256 px, where there is room for the mark the product is known by.
fn level_of(image: &RgbaImage) -> Level {
    let fill = pixel(image, FILL_PROBE.0, FILL_PROBE.1);
    assert!(fill[3] > 200, "the image is transparent where the tile should be");
    // Measured: 431 on the filled tile, 98 on the plate. The threshold sits
    // between them with room on both sides, so antialiasing cannot flip it.
    if brightness(fill) > 260 {
        return Level::Small;
    }
    // Measured: 280 on the bracket from 32 px up, 98 on bare plate.
    if brightness(pixel(image, METAPHOR_PROBE.0, METAPHOR_PROBE.1)) > 180 {
        Level::Large
    } else {
        Level::Medium
    }
}

fn decode(png: &[u8], what: &str) -> RgbaImage {
    image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .unwrap_or_else(|e| panic!("{what} is not a PNG: {e}"))
        .to_rgba8()
}

/// The colour a fill in a master SVG is painted with, as RGB.
fn master_fill(path: &str) -> [u8; 3] {
    let svg = read_text(path);
    let hex = svg
        .split("fill=\"#")
        .nth(1)
        .map(|rest| &rest[..6])
        .unwrap_or_else(|| panic!("{path} paints no fill"));
    let channel = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).expect("a hex colour");
    [channel(0), channel(2), channel(4)]
}

fn close_to(rgba: [u8; 4], rgb: [u8; 3]) -> bool {
    rgba.iter().zip(rgb).all(|(have, want)| have.abs_diff(want) <= 6)
}

/// One image inside the `.ico`, as its directory describes it.
struct Entry {
    size: u32,
    png: Vec<u8>,
}

fn ico_entries() -> Vec<Entry> {
    let ico = read("assets/icon.ico");
    assert_eq!(u16::from_le_bytes([ico[2], ico[3]]), 1, "icon.ico is not an icon resource");
    let count = usize::from(u16::from_le_bytes([ico[4], ico[5]]));
    (0..count)
        .map(|index| {
            let at = 6 + index * 16;
            // A zero width means 256: the field is one byte and 256 does not fit.
            let size = if ico[at] == 0 { 256 } else { u32::from(ico[at]) };
            let field = |from: usize| u32::from_le_bytes([ico[at + from], ico[at + from + 1], ico[at + from + 2], ico[at + from + 3]]) as usize;
            let (length, offset) = (field(8), field(12));
            let png = ico
                .get(offset..offset + length)
                .unwrap_or_else(|| panic!("the {size}px entry points past the end of icon.ico"))
                .to_vec();
            Entry { size, png }
        })
        .collect()
}

#[test]
fn every_size_in_the_icon_carries_the_level_that_reads_at_it() {
    // S at 27 px and below, where the outline and the chevrons collapse into
    // noise and the filled tile is all that reads; M to 63; L above.
    for entry in ico_entries() {
        let image = decode(&entry.png, &format!("the {}px entry", entry.size));
        assert_eq!(image.width(), entry.size, "the {}px entry holds a {}px image", entry.size, image.width());
        assert_eq!(
            level_of(&image),
            level_for(entry.size),
            "the {}px icon is cut from the wrong master; the line's rule is S up to 27px, M to 63px, L above",
            entry.size
        );
    }
}

#[test]
fn the_largest_icon_image_comes_first() {
    // Windows picks by closest size and ignores order, but readers that take
    // the first entry verbatim exist - a sibling project's title bar was
    // stretched from a 16px entry for exactly this reason.
    let sizes: Vec<u32> = ico_entries().iter().map(|entry| entry.size).collect();
    let mut descending = sizes.clone();
    descending.sort_unstable_by(|a, b| b.cmp(a));
    assert_eq!(sizes, descending, "icon.ico does not list its images largest first");
    assert!(
        sizes.contains(&256) && sizes.contains(&16),
        "icon.ico does not span the sizes Windows asks for: {sizes:?}"
    );
}

#[test]
fn all_three_masters_reach_the_icon() {
    // The rule above holds vacuously for a container of small sizes only.
    let levels: Vec<Level> = ico_entries().iter().map(|entry| level_of(&decode(&entry.png, "an entry"))).collect();
    for level in [Level::Small, Level::Medium, Level::Large] {
        assert!(levels.contains(&level), "icon.ico carries no {level:?} image: {levels:?}");
    }
}

#[test]
fn the_rasters_keep_the_colours_of_the_masters() {
    // A raster that passes the level rule but was drawn in another hue - a
    // stale export after a recolour, say - is still the wrong mark.
    let gold = master_fill(S);
    let plate = master_fill(L);
    for entry in ico_entries() {
        let image = decode(&entry.png, "an entry");
        let probe = pixel(&image, FILL_PROBE.0, FILL_PROBE.1);
        let (name, want) = if level_for(entry.size) == Level::Small {
            ("product colour", gold)
        } else {
            ("plate", plate)
        };
        assert!(
            close_to(probe, want),
            "the {}px icon's {name} is {probe:?}, the master paints {want:?}",
            entry.size
        );
    }
}

#[test]
fn the_square_rasters_are_cut_for_their_size() {
    // 180 px is a home-screen tile, not an icon in a list: the full mark. The
    // line shipped it cut from S for months, a gold square on a phone.
    for (path, size) in [
        ("assets/favicon-32.png", 32),
        ("assets/apple-touch-icon.png", 180),
        ("assets/logo-512.png", 512),
    ] {
        let image = decode(&read(path), path);
        assert_eq!(image.dimensions(), (size, size), "{path} is not {size}x{size}");
        assert_eq!(level_of(&image), level_for(size), "{path} is cut from the wrong master");
    }
}

#[test]
fn the_social_preview_is_the_banner_on_its_plate() {
    // GitHub's social preview and the og:image both sites point at.
    let image = decode(&read("assets/social-preview.png"), "the social preview");
    assert_eq!(image.dimensions(), (1280, 640), "GitHub asks for a 1280x640 social preview");

    // Laid on the plate colour, so no seam of a rounded banner shows at the
    // edges.
    let plate = master_fill(L);
    for (x, y) in [(0, 0), (1279, 0), (0, 639), (1279, 639), (640, 20), (640, 619)] {
        assert!(
            close_to(image.get_pixel(x, y).0, plate),
            "the social preview's edge at {x},{y} is not the plate"
        );
    }

    // And centred. The banner is a wide plate with its artwork on the left,
    // so laid down as it is the artwork lands off-centre - a sibling's preview
    // shipped that way. The box around everything that is not plate must have
    // equal margins, to within the slack of antialiased edges.
    let drawn: Vec<(u32, u32)> = image
        .enumerate_pixels()
        .filter(|(_, _, rgba)| !close_to(rgba.0, plate))
        .map(|(x, y, _)| (x, y))
        .collect();
    let (left, right) = (drawn.iter().map(|p| p.0).min().unwrap(), drawn.iter().map(|p| p.0).max().unwrap());
    let (top, bottom) = (drawn.iter().map(|p| p.1).min().unwrap(), drawn.iter().map(|p| p.1).max().unwrap());
    let (margin_right, margin_bottom) = (1279 - right, 639 - bottom);
    assert!(
        left.abs_diff(margin_right) <= 8 && top.abs_diff(margin_bottom) <= 8,
        "the artwork is off-centre: margins left {left}, right {margin_right}, top {top}, bottom {margin_bottom}"
    );
    // Large enough to read in a feed, and clear of the edges GitHub crops.
    assert!(right - left >= 800, "the artwork is only {}px wide", right - left);
    assert!(left >= 64 && top >= 64, "the artwork runs into the edge GitHub crops");
}

#[test]
fn the_sites_carry_the_same_mark_as_the_repository() {
    // The docs site and the web UI keep their own copies - Astro and Vite
    // want them under their own directories - and copies go stale. The
    // pairing also fixes the level per placement: the docs header takes L,
    // a favicon S. Swapping them puts a solid gold block in the header,
    // which is exactly what shipped in kasl.
    for (copy, original) in [
        ("docs/src/assets/logo.svg", L),
        ("docs/public/favicon.svg", S),
        ("frontend/public/favicon.svg", S),
        ("docs/public/apple-touch-icon.png", "assets/apple-touch-icon.png"),
        ("frontend/public/apple-touch-icon.png", "assets/apple-touch-icon.png"),
    ] {
        assert!(read(copy) == read(original), "{copy} differs from {original}; re-run docs/export-assets.mjs");
    }
    // Not a copy of M by accident: the medium master is still in use by name.
    assert_ne!(read(M), read(S));
}

/// Every `raw.githubusercontent.com` picture a page names, as a path in this
/// repository.
fn pictures_on_github(text: &str) -> Vec<String> {
    const PREFIX: &str = "https://raw.githubusercontent.com/lacodda/kasl-server/main/";
    text.split(PREFIX)
        .skip(1)
        .map(|rest| rest[..rest.find(['"', '\'']).unwrap_or(rest.len())].to_string())
        .collect()
}

#[test]
fn every_page_unfurls_into_a_picture_that_exists() {
    // og:image is absolute, so a renamed file breaks every unfurl at once and
    // nothing in either build says so.
    for page in ["frontend/index.html", "docs/astro.config.mjs"] {
        let pictures = pictures_on_github(&read_text(page));
        assert!(
            pictures.iter().any(|path| path == "assets/social-preview.png"),
            "{page} does not unfurl into the social preview"
        );
        for path in pictures {
            assert!(repo_root().join(&path).is_file(), "{page} names {path}, which is not in the repository");
        }
    }
}

#[test]
fn every_picture_the_readme_shows_is_in_the_repository() {
    // The README addresses its pictures absolutely, so it renders on
    // crates.io too - and an absolute URL to a renamed file is a broken image
    // on every storefront at once, with nothing in a build to say so.
    const PREFIX: &str = "https://github.com/lacodda/kasl-server/raw/main/";
    let readme = read_text("README.md");
    let pictures: Vec<&str> = readme.split(PREFIX).skip(1).map(|rest| &rest[..rest.find('"').unwrap_or(rest.len())]).collect();
    assert!(pictures.contains(&"assets/banner.svg"), "the README lost its banner");
    assert!(pictures.contains(&"assets/screenshot.png"), "the README lost its screenshot");
    for path in pictures {
        assert!(repo_root().join(path).is_file(), "the README shows {path}, which is not in the repository");
    }
}

#[test]
fn the_web_ui_offers_the_icons_it_points_at() {
    // A `<link rel>` to a file that is not there is a broken icon on every
    // installation at once.
    let html = read_text("frontend/index.html");
    for (href, file) in [
        ("href=\"/favicon.svg\"", "frontend/public/favicon.svg"),
        ("href=\"/apple-touch-icon.png\"", "frontend/public/apple-touch-icon.png"),
    ] {
        assert!(html.contains(href), "frontend/index.html does not link {href}");
        assert!(repo_root().join(file).is_file(), "frontend/index.html links {href} but {file} is missing");
    }
}

#[test]
fn the_package_carries_the_icon() {
    // build.rs embeds it unconditionally on Windows, so `cargo install` there
    // needs the file inside the package - and `include` is an explicit list.
    let manifest = read_text("Cargo.toml");
    assert!(manifest.contains("\"assets/icon.ico\""), "Cargo.toml's include list leaves out assets/icon.ico");
}

#[cfg(windows)]
#[test]
fn the_windows_executable_carries_the_icon() {
    // The file being right is half of it; the other half is that something
    // reads it. A console program without an icon resource gets Windows'
    // generic one, and nobody looking at the repository would know. The
    // resource compiler stores PNG-compressed entries verbatim, so the 256px
    // image is found byte for byte in the binary built for this test run.
    let largest = ico_entries().into_iter().max_by_key(|entry| entry.size).expect("icon.ico has images");
    let binary = fs::read(env!("CARGO_BIN_EXE_kasl-server")).expect("the test binary's sibling executable exists");
    let needle = &largest.png;
    let found = binary
        .windows(16)
        .enumerate()
        .filter(|(_, window)| *window == &needle[..16])
        .any(|(at, _)| binary.get(at..at + needle.len()) == Some(needle.as_slice()));
    assert!(found, "kasl-server.exe does not carry the 256px icon; build.rs did not embed assets/icon.ico");
}
