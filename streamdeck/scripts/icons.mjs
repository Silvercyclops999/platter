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
/** `shapes` in 144 units: { ring: [r, width] } or { dot: r }, all centred. */
function png(size, shapes) {
  const scale = size / 144;
  const rows = [];
  for (let y = 0; y < size; y++) {
    const row = Buffer.alloc(1 + size * 4);
    for (let x = 0; x < size; x++) {
      const d = Math.hypot(x + 0.5 - size / 2, y + 0.5 - size / 2) / scale;
      let a = 0;
      for (const s of shapes) {
        const edge = s.dot !== undefined ? s.dot - d : s.ring[1] / 2 - Math.abs(d - s.ring[0]);
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
const plugin = [{ ring: [56, 8] }, { ring: [36, 3] }, { dot: 10 }];
const category = [{ ring: [56, 10] }, { dot: 14 }];
for (const [name, shapes, size] of [
  ["plugin/icon", plugin, 256],
  ["plugin/icon@2x", plugin, 512],
  ["plugin/category", category, 28],
  ["plugin/category@2x", category, 56],
]) {
  writeFileSync(join(root, `${name}.png`), png(size, shapes));
}
console.log(`wrote ${Object.keys(icons).length * 2} icons and 4 PNGs`);
