// Rebuilds the bundled headphone correction index from AutoEq's results list
// (github.com/jaakkopasanen/AutoEq, MIT). Run: node scripts/autoeq-index.mjs
// Only names and folders are kept; profiles are fetched when someone picks one.
import { writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const SOURCE = "https://raw.githubusercontent.com/jaakkopasanen/AutoEq/master/results/INDEX.md";
const OUT = fileURLToPath(new URL("../src-tauri/src/audio/autoeq_index.txt", import.meta.url));

const text = await (await fetch(SOURCE)).text();
const byDir = new Map();
for (const line of text.split("\n")) {
  const m = /^- \[(.+)\]\(\.\/(.+)\) by (.+)$/.exec(line.trim());
  if (!m) continue;
  const path = decodeURIComponent(m[2]);
  const cut = path.lastIndexOf("/");
  const dir = path.slice(0, cut);
  const name = path.slice(cut + 1);
  if (name.includes("\t") || dir.includes("\t")) continue;
  if (!byDir.has(dir)) byDir.set(dir, { label: m[3].trim(), names: new Set() });
  byDir.get(dir).names.add(name);
}

const out = ["# Headphone correction index from AutoEq (MIT). Rebuilt by scripts/autoeq-index.mjs."];
let count = 0;
for (const dir of [...byDir.keys()].sort()) {
  const { label, names } = byDir.get(dir);
  out.push(`@${dir}\t${label}`);
  for (const n of [...names].sort((a, b) => a.localeCompare(b))) {
    out.push(n);
    count++;
  }
}
writeFileSync(OUT, out.join("\n") + "\n");
console.log(`${count} profiles in ${byDir.size} folders`);
