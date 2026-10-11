// Rasterize the kasl-server brand SVGs into the PNGs and the multi-size .ico,
// and lay the copies out where the docs site and the web UI serve them.
//
// Run from this package, so ESM resolves the sharp it depends on:
//   node export-assets.mjs
//
// The SVG masters in ../assets are exports of the line's brand generator and
// are never edited here; everything this script writes is derived from them
// and is committed, so neither site needs Node to show the mark.
import sharp from "sharp";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ASSETS = path.resolve(HERE, "../assets");
const DOCS_PUBLIC = path.join(HERE, "public");
const DOCS_ASSETS = path.join(HERE, "src/assets");
const WEB_PUBLIC = path.resolve(HERE, "../frontend/public");

// The tile everything sits on, the same colour as the plate inside the masters.
const PLATE = "#1B2126";

// Three masters, one per level of detail. Which one a raster comes from is
// decided by how big that raster is, never by what it is for: an .ico holds
// sizes from 16 to 256, and there is room in it for all three.
const S = path.join(ASSETS, "logo-s.svg"); // filled hex, bold code
const M = path.join(ASSETS, "logo-m.svg"); // plated hex, code alone
const L = path.join(ASSETS, "logo.svg"); // the full mark: code and metaphor
const BANNER = path.join(ASSETS, "banner.svg");

/**
 * The master a raster of this size should be cut from.
 *
 * S is a hex filled with colour: at 16 px that is the only thing that reads,
 * and at 48 px it is a coloured blob where the mark should be. L carries the
 * chevrons under the code: legible large, mud when small. The bands are the
 * line's brand canon; tests/brand_assets.rs reads the same rule back off the
 * pixels this script writes.
 */
function levelFor(size) {
  if (size <= 27) return S;
  if (size <= 63) return M;
  return L;
}

// Largest first. Windows picks by nearest size and ignores the order, but
// readers that take the first entry verbatim exist, and a 16 px entry at the
// front is a title bar stretched from sixteen pixels.
const ICO_SIZES = [256, 128, 64, 48, 32, 24, 16];

function raster(src, size) {
  return sharp(src, { density: 384 }).resize(size, size).png().toBuffer();
}

// sharp cannot write an .ico, so the container is assembled here: a 6-byte
// header, a 16-byte directory entry per image, then the PNG payloads. Done by
// hand on purpose - writers that take a list of sizes (Pillow's among them)
// sort it ascending, which is exactly the order the comment above rules out.
function buildIco(images) {
  const header = Buffer.alloc(6);
  header.writeUInt16LE(0, 0); // reserved
  header.writeUInt16LE(1, 2); // type: icon
  header.writeUInt16LE(images.length, 4);

  const directory = Buffer.alloc(16 * images.length);
  let offset = header.length + directory.length;
  images.forEach(({ size, png }, i) => {
    const at = 16 * i;
    // One byte per dimension, so 256 is written as 0.
    directory.writeUInt8(size >= 256 ? 0 : size, at + 0);
    directory.writeUInt8(size >= 256 ? 0 : size, at + 1);
    directory.writeUInt8(0, at + 2); // no palette
    directory.writeUInt8(0, at + 3); // reserved
    directory.writeUInt16LE(1, at + 4); // colour planes
    directory.writeUInt16LE(32, at + 6); // bits per pixel
    directory.writeUInt32LE(png.length, at + 8);
    directory.writeUInt32LE(offset, at + 12);
    offset += png.length;
  });

  return Buffer.concat([header, directory, ...images.map(({ png }) => png)]);
}

// The Windows executable's icon: build.rs embeds this file into the binary.
const icoImages = [];
for (const size of ICO_SIZES) {
  icoImages.push({ size, png: await raster(levelFor(size), size) });
}
fs.writeFileSync(path.join(ASSETS, "icon.ico"), buildIco(icoImages));
console.log("wrote assets/icon.ico");

fs.writeFileSync(path.join(ASSETS, "favicon-32.png"), await raster(levelFor(32), 32));
// 180 px is a home-screen tile, not an icon in a list: the full mark.
fs.writeFileSync(path.join(ASSETS, "apple-touch-icon.png"), await raster(levelFor(180), 180));
fs.writeFileSync(path.join(ASSETS, "logo-512.png"), await raster(levelFor(512), 512));
console.log("wrote assets/favicon-32.png, apple-touch-icon.png, logo-512.png");

// The social preview: what a link to the repository or the docs unfurls into,
// and the picture GitHub shows for the repository. 1280x640 is GitHub's size.
//
// The banner is a plate with the artwork on its left: drawn as it is, the
// artwork would land off-centre and the plate's rounded corners would show as
// a seam against a background of the same colour. So the banner is laid on
// that colour first - the corners vanish into it - then trimmed down to the
// artwork, wherever the fonts installed here put the wordmark's end, and
// centred. Nothing about the banner's geometry is hard-coded.
const PREVIEW = { width: 1280, height: 640 };
// GitHub crops the edges in some views; keep the artwork inside this margin.
const PREVIEW_MARGIN = 140;

const bannerOnPlate = await sharp(BANNER, { density: 384 }).flatten({ background: PLATE }).png().toBuffer();
// The hex's own fill is a shade off the plate; a low threshold keeps it from
// counting as background while still eating the antialiased plate edge.
const artwork = await sharp(bannerOnPlate).trim({ background: PLATE, threshold: 4 }).png().toBuffer();
const fitted = await sharp(artwork)
  .resize({
    width: PREVIEW.width - 2 * PREVIEW_MARGIN,
    height: PREVIEW.height - 2 * PREVIEW_MARGIN,
    fit: "inside",
  })
  .png()
  .toBuffer();

await sharp({ create: { ...PREVIEW, channels: 4, background: PLATE } })
  .composite([{ input: fitted, gravity: "centre" }])
  .png()
  .toFile(path.join(ASSETS, "social-preview.png"));
console.log("wrote assets/social-preview.png");

// The copies each site serves. Written from the same masters by the same rule
// and checked byte for byte by tests/brand_assets.rs, so the docs site, the
// web UI and the repository cannot drift apart.
for (const dir of [DOCS_PUBLIC, DOCS_ASSETS, WEB_PUBLIC]) {
  fs.mkdirSync(dir, { recursive: true });
}
// A favicon is drawn at 16 px in a tab, whatever its viewBox says.
fs.copyFileSync(S, path.join(DOCS_PUBLIC, "favicon.svg"));
fs.copyFileSync(S, path.join(WEB_PUBLIC, "favicon.svg"));
// The docs header takes the full mark: the line's canon for that placement.
fs.copyFileSync(L, path.join(DOCS_ASSETS, "logo.svg"));
fs.copyFileSync(path.join(ASSETS, "apple-touch-icon.png"), path.join(DOCS_PUBLIC, "apple-touch-icon.png"));
fs.copyFileSync(path.join(ASSETS, "apple-touch-icon.png"), path.join(WEB_PUBLIC, "apple-touch-icon.png"));
console.log("copied the site assets into docs/ and frontend/public/");
