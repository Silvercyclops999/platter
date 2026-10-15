// Packs the built plugin folder into dist/app.platter.streamdeck.streamDeckPlugin
// with the Stream Deck CLI (which validates it first).
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const folder = join(root, "app.platter.streamdeck.sdPlugin");
if (!existsSync(join(folder, "bin", "plugin.js"))) {
  console.error("Build first: npm run build");
  process.exit(1);
}
const out = join(root, "dist");
mkdirSync(out, { recursive: true });
const cli = join(root, "node_modules", "@elgato", "cli", "bin", "streamdeck.mjs");
execFileSync(process.execPath, [cli, "pack", folder, "--output", out, "--force", "--no-update-check"], { stdio: "inherit", cwd: root });
