// Bundles src/plugin.ts into the plugin folder's bin/plugin.js.
import { build } from "esbuild";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

await build({
  entryPoints: [join(root, "src", "plugin.ts")],
  outfile: join(root, "app.platter.streamdeck.sdPlugin", "bin", "plugin.js"),
  bundle: true,
  platform: "node",
  target: "node20",
  format: "esm",
  sourcemap: false,
  logLevel: "info",
  // Some dependencies still call require(); give the ESM bundle one.
  banner: { js: "import { createRequire } from 'node:module'; const require = createRequire(import.meta.url);" },
});
