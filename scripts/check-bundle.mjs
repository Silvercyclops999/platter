// Checks the built UI (dist/) before it goes into an app: no source maps, and
// nothing that looks like a token, key, sign-in or a real home server. Runs at
// the end of `npm run build`, so every release build passes through it.
//
//   node scripts/check-bundle.mjs [folder]   (default: dist)
//
// When a .env sits next to package.json (a developer's own machine), none of
// its values may show up in the bundle either.
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const dir = path.resolve(root, process.argv[2] ?? "dist");

/** Things that never belong in the shipped UI, with what each one is. */
const RULES = [
  [/\/\/[#@] sourceMappingURL=/, "a source map link"],
  [/x-plex-token=[A-Za-z0-9_-]{8,}/i, "a Plex token in an address"],
  [/[0-9a-f]{32}\.plex\.direct/i, "a real plex.direct server name"],
  [/(?<![\d.])192\.168\.\d{1,3}\.\d{1,3}(?![\d.])/, "a home network address"],
  [/-----BEGIN [A-Z ]*PRIVATE KEY-----/, "a private key"],
  [/\b(?:gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{40,})/, "a GitHub token"],
  [/\bsk-ant-[A-Za-z0-9_-]{20,}/, "an Anthropic key"],
  [/\bAKIA[0-9A-Z]{16}\b/, "an AWS key"],
  [/\bAIza[0-9A-Za-z_-]{35}\b/, "a Google key"],
  [/\bxox[abprs]-[A-Za-z0-9-]{10,}/, "a Slack token"],
  [/[?&]apikey=[A-Za-z0-9]{20,}/i, "an API key in an address"],
  [/\b(?:postgres(?:ql)?|mysql|mongodb(?:\+srv)?|redis):\/\/[^\s"'`]*:[^\s"'`@]+@/, "a database address with a password"],
  [/[A-Za-z0-9._%+-]+@users\.noreply\.github\.com/, "a GitHub email"],
];

function files(d) {
  return readdirSync(d).flatMap((name) => {
    const p = path.join(d, name);
    return statSync(p).isDirectory() ? files(p) : [p];
  });
}

/** Values from the developer's .env, long enough to mean something. */
function envValues() {
  const env = path.join(root, ".env");
  if (!existsSync(env)) return [];
  return readFileSync(env, "utf8")
    .split(/\r?\n/)
    .map((l) => l.match(/^\s*[A-Za-z_][A-Za-z0-9_]*\s*=\s*(.*)$/)?.[1]?.trim().replace(/^(["'])(.*)\1$/, "$2") ?? "")
    .filter((v) => v.length >= 8);
}

if (!existsSync(dir)) {
  console.error(`check-bundle: ${dir} doesn't exist; build first`);
  process.exit(1);
}

const problems = [];
const secrets = envValues();
for (const file of files(dir)) {
  const rel = path.relative(root, file);
  if (file.endsWith(".map")) {
    problems.push(`${rel}: a source map file`);
    continue;
  }
  if (!/\.(js|mjs|css|html|json|svg|txt|webmanifest)$/.test(file)) continue;
  const text = readFileSync(file, "utf8");
  for (const [re, what] of RULES) {
    if (re.test(text)) problems.push(`${rel}: ${what}`);
  }
  if (secrets.some((s) => text.includes(s))) problems.push(`${rel}: a value from .env`);
}

if (problems.length) {
  console.error("check-bundle: the built UI holds something it must not:");
  for (const p of problems) console.error(`  ${p}`);
  process.exit(1);
}
console.log(`check-bundle: ${dir} is clean`);
