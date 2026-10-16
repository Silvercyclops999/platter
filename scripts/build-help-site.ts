// Builds the public docs site from docs/help into the folder given (default
// "site"), failing on any broken link. Run with:
//   node --experimental-strip-types scripts/build-help-site.ts [out]
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { brokenLinks, buildSite } from "../src/help/site.ts";

const root = join(import.meta.dirname, "..");
const help = join(root, "docs", "help");
const out = resolve(root, process.argv[2] ?? "site");

const pages = readdirSync(help)
  .filter((f) => f.endsWith(".md"))
  .map((f) => ({ slug: f.slice(0, -3), md: readFileSync(join(help, f), "utf8") }));
const files = buildSite(pages);
const imageDir = join(help, "images");
const images = existsSync(imageDir) ? readdirSync(imageDir).map((f) => `images/${f}`) : [];
const broken = brokenLinks(files, new Set(images));
if (broken.length) {
  console.error(`Broken links:\n${broken.join("\n")}`);
  process.exit(1);
}
mkdirSync(join(out, "images"), { recursive: true });
for (const [path, text] of files) writeFileSync(join(out, path), text);
for (const img of images) copyFileSync(join(help, img), join(out, img));
console.log(`Built ${pages.length} help pages into ${out}`);
