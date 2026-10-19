// Pins the Bridge's Unraid template and compose file to the app's version,
// so Unraid installs a release, never whatever is on main.
//
//   node scripts/pin-bridge-template.mjs          rewrite both to package.json's version
//   node scripts/pin-bridge-template.mjs --check  fail when either names another version (CI)
//
// Run it with every version bump, before tagging vX.Y.Z. The template's
// TemplateURL then points at the tag, and its image at platter-bridge:X.Y.Z,
// which the bridge workflow pushes for that tag.
import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const version = JSON.parse(readFileSync(path.join(root, "package.json"), "utf8")).version;
const check = process.argv.includes("--check");

const image = /(ghcr\.io\/silvercyclops999\/platter-bridge):[^<"\s]+/g;
const templateUrl = /(raw\.githubusercontent\.com\/Silvercyclops999\/platter\/)[^/]+(\/docker\/bridge\/platter-bridge\.xml)/g;

const files = ["docker/bridge/platter-bridge.xml", "docker/bridge/docker-compose.yml"];
let wrong = 0;
for (const f of files) {
  const p = path.join(root, f);
  const was = readFileSync(p, "utf8");
  // The image follows :latest and the template's own address follows main, so
  // Unraid (and Watchtower) see each new release and a template refresh never
  // pins an old one.
  const now = was.replace(image, "$1:latest").replace(templateUrl, "$1main$2");
  if (!/ghcr\.io\/silvercyclops999\/platter-bridge:/.test(was)) {
    console.error(`${f}: no platter-bridge image to pin`);
    wrong++;
  }
  if (now !== was) {
    if (check) {
      console.error(`${f} should follow main and :latest. Run: node scripts/pin-bridge-template.mjs`);
      wrong++;
    } else {
      writeFileSync(p, now);
      console.log(`${f}: follows main and :latest`);
    }
  }
}
process.exit(wrong ? 1 : 0);
