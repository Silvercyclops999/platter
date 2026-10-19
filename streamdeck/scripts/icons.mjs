// Writes the plugin's icons from the glyphs in scripts/glyphs.mjs: the
// action list icons (white on transparent) and each key's starting picture
// (the glyph on Platter's dark tile). Run with `node scripts/icons.mjs`
// after changing a glyph.
import { mkdirSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { GLYPHS } from "./glyphs.mjs";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "app.platter.streamdeck.sdPlugin", "imgs");
const W = "#fff";
const TILE = "#171513";
const TEXT = "#F4EFE8";

const svg = (body) => `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 144 144" width="144" height="144">${body}</svg>\n`;
const put = (name, body) => {
  const file = join(root, `${name}.svg`);
  mkdirSync(dirname(file), { recursive: true });
  writeFileSync(file, svg(body));
};

// Start clean so a renamed glyph leaves nothing behind.
for (const dir of ["actions", "keys"]) {
  try {
    for (const f of readdirSync(join(root, dir))) if (f.endsWith(".svg")) rmSync(join(root, dir, f));
  } catch {
    /* not there yet */
  }
}
for (const [name, draw] of Object.entries(GLYPHS)) {
  put(`actions/${name}`, draw(W));
  put(`keys/${name}`, `<rect width="144" height="144" fill="${TILE}"/><g transform="translate(72 72) scale(0.66) translate(-72 -72)">${draw(TEXT)}</g>`);
}
const icons = GLYPHS;
// The plugin and category icons must also be PNG: rings and a centre dot,
// drawn here with simple anti-aliasing, white on transparent.
import { deflateSync } from "node:zlib";

const CRC = new Uint32Array(256).map((_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
function crc32(buf) {
  let c = 0xffffffff;
  for (const b of buf) c = CRC[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}
function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const td = Buffer.concat([Buffer.from(type), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(td));
  return Buffer.concat([len, td, crc]);
}
/** Distance from p to the segment a-b. */
function segment(px, py, [ax, ay, bx, by]) {
  const dx = bx - ax;
  const dy = by - ay;
  const t = Math.max(0, Math.min(1, ((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy)));
  return Math.hypot(px - ax - t * dx, py - ay - t * dy);
}

/**
 * How far inside a shape a point is (negative outside), in 144 units.
 * Shapes: { ring: [r, width] } or { dot: r } centred; { oval: [cx, cy, rx, ry, width] }
 * an ellipse outline and { fill: [cx, cy, rx, ry] } a filled one; { line: [x1, y1, x2, y2, width] };
 * { ringAt: [cx, cy, r, width] } and { dotAt: [cx, cy, r] } off centre; { rect: [x0, y0, x1, y1] }.
 */
function inside(s, px, py) {
  if (s.ringAt) return s.ringAt[3] / 2 - Math.abs(Math.hypot(px - s.ringAt[0], py - s.ringAt[1]) - s.ringAt[2]);
  if (s.dotAt) return s.dotAt[2] - Math.hypot(px - s.dotAt[0], py - s.dotAt[1]);
  if (s.rect) return Math.min(px - s.rect[0], s.rect[2] - px, py - s.rect[1], s.rect[3] - py);
  if (s.dot !== undefined) return s.dot - Math.hypot(px - 72, py - 72);
  if (s.ring) return s.ring[1] / 2 - Math.abs(Math.hypot(px - 72, py - 72) - s.ring[0]);
  if (s.line) return s.line[4] / 2 - segment(px, py, s.line);
  const [cx, cy, rx, ry, w] = s.oval ?? [...s.fill, 0];
  // Distance to the ellipse, near enough for edge smoothing.
  const n = Math.hypot((px - cx) / rx, (py - cy) / ry);
  const d = (n - 1) * Math.min(rx, ry);
  return s.oval ? w / 2 - Math.abs(d) : -d;
}

/** White on transparent, from `shapes` in 144 units. */
function png(size, shapes) {
  const scale = size / 144;
  const rows = [];
  for (let y = 0; y < size; y++) {
    const row = Buffer.alloc(1 + size * 4);
    for (let x = 0; x < size; x++) {
      const px = (x + 0.5) / scale;
      const py = (y + 0.5) / scale;
      let a = 0;
      for (const s of shapes) {
        const edge = inside(s, px, py);
        a = Math.max(a, Math.min(1, Math.max(0, edge * scale + 0.5)));
      }
      row.set([255, 255, 255, Math.round(a * 255)], 1 + x * 4);
    }
    rows.push(row);
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(size, 0);
  ihdr.writeUInt32BE(size, 4);
  ihdr.set([8, 6, 0, 0, 0], 8);
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(Buffer.concat(rows))),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}
// Platter's mark (src-tauri/app-icon.svg): a P whose bowl is a record, a
// ring with the spindle in its centre. The 1024 drawing scaled by 0.19.
const plugin = [
  { ringAt: [72, 51.5, 28.1, 22.8] },
  { rect: [32.5, 51.5, 54.5, 132] },
  { dotAt: [72, 51.5, 5.7] },
];
// The category icon is drawn at 28 px: no spindle, which would only blur.
const category = [
  { ringAt: [72, 51.5, 28.1, 22.8] },
  { rect: [32.5, 51.5, 54.5, 132] },
];
for (const [name, shapes, size] of [
  ["plugin/icon", plugin, 256],
  ["plugin/icon@2x", plugin, 512],
  ["plugin/category", category, 28],
  ["plugin/category@2x", category, 56],
]) {
  writeFileSync(join(root, `${name}.png`), png(size, shapes));
}
console.log(`wrote ${Object.keys(icons).length * 2} icons and 4 PNGs`);
