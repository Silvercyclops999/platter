// Writes the plugin's monochrome key and list icons (simple shapes, white on
// transparent). Run with `node scripts/icons.mjs` after changing one.
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "app.platter.streamdeck.sdPlugin", "imgs");
const W = "#fff";

const heart = "M72 112 34 74a22 22 0 0 1 38-30 22 22 0 0 1 38 30z";
const icons = {
  "actions/play": `<path d="M52 38v68l56-34z" fill="${W}"/>`,
  "actions/pause": `<rect x="44" y="38" width="20" height="68" rx="4" fill="${W}"/><rect x="80" y="38" width="20" height="68" rx="4" fill="${W}"/>`,
  "actions/next": `<path d="M40 40v64l48-32z" fill="${W}"/><rect x="92" y="40" width="12" height="64" rx="3" fill="${W}"/>`,
  "actions/previous": `<path d="M104 40v64L56 72z" fill="${W}"/><rect x="40" y="40" width="12" height="64" rx="3" fill="${W}"/>`,
  "actions/heart": `<path d="${heart}" fill="none" stroke="${W}" stroke-width="8" stroke-linejoin="round"/>`,
  "actions/heart-filled": `<path d="${heart}" fill="${W}" stroke="${W}" stroke-width="8" stroke-linejoin="round"/>`,
  "actions/volume": `<path d="M30 58h20l26-22v72L50 86H30z" fill="${W}"/><path d="M90 54a26 26 0 0 1 0 36M100 42a42 42 0 0 1 0 60" fill="none" stroke="${W}" stroke-width="8" stroke-linecap="round"/>`,
  "actions/request": `<circle cx="66" cy="66" r="40" fill="none" stroke="${W}" stroke-width="8"/><circle cx="66" cy="66" r="8" fill="${W}"/><path d="M110 92v32M94 108h32" stroke="${W}" stroke-width="8" stroke-linecap="round"/>`,
  "actions/listen": `<path d="M32 92V76a40 40 0 0 1 80 0v16" fill="none" stroke="${W}" stroke-width="8"/><rect x="26" y="84" width="22" height="34" rx="6" fill="${W}"/><rect x="96" y="84" width="22" height="34" rx="6" fill="${W}"/>`,
};

for (const [name, body] of Object.entries(icons)) {
  const file = join(root, `${name}.svg`);
  mkdirSync(dirname(file), { recursive: true });
  writeFileSync(file, `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 144 144" width="144" height="144">${body}</svg>\n`);
}
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
console.log(`wrote ${Object.keys(icons).length} icons and 4 PNGs`);
