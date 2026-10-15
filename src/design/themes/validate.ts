/**
 * Theme file rules, shared with the core (src-tauri/src/themes.rs reads the
 * same contract.json). A theme file is data only: a name, a base (dark or
 * light) and token values. Only known token names are taken, and only plain
 * colours or pixel lengths: no CSS, no url(), no scripts.
 */
import contract from "./contract.json";

export type ThemeBase = "dark" | "light";
export type TokenKind = "colour" | "length";

/** Every token a theme may set, with the kind of value it takes. */
export const THEME_TOKENS = contract.tokens as Record<string, TokenKind>;
export const THEME_TOKEN_NAMES = Object.keys(THEME_TOKENS);

/** A validated theme file (also what the Theme file setting holds). */
export interface UserTheme {
  /** The file's name in the themes folder. */
  file: string;
  name: string;
  base: ThemeBase;
  tokens: Record<string, string>;
}

export type ThemeCheck = { ok: true; theme: UserTheme } | { ok: false; reason: string };

export const THEME_FILE_MAX_BYTES = 64 * 1024;
export const THEME_NAME_MAX = 60;
const VALUE_MAX = 64;
const FIELDS = ["name", "base", "tokens", "author"];

const NUM = String.raw`\d{1,3}(?:\.\d+)?`;
const HEX = /^#(?:[0-9a-f]{3}|[0-9a-f]{4}|[0-9a-f]{6}|[0-9a-f]{8})$/i;
const RGB = new RegExp(
  String.raw`^rgba?\(\s*${NUM}%?\s*[,\s]\s*${NUM}%?\s*[,\s]\s*${NUM}%?\s*(?:[,/]\s*${NUM}%?\s*)?\)$`,
  "i",
);
const HSL = new RegExp(
  String.raw`^hsla?\(\s*${NUM}(?:deg)?\s*[,\s]\s*${NUM}%\s*[,\s]\s*${NUM}%\s*(?:[,/]\s*${NUM}%?\s*)?\)$`,
  "i",
);
const LENGTH = /^(?:0|\d{1,3}(?:\.\d+)?px)$/;

export function isColourValue(v: string): boolean {
  return HEX.test(v) || RGB.test(v) || HSL.test(v);
}

export function isLengthValue(v: string): boolean {
  return LENGTH.test(v);
}

/** Why a token value isn't allowed, or null when it's fine. */
export function tokenValueProblem(name: string, value: unknown): string | null {
  const kind = THEME_TOKENS[name];
  if (typeof value !== "string") return `The value of "${name}" isn't text.`;
  const v = value.trim();
  if (/url\s*\(/i.test(v)) return `"${name}" uses url(). Theme files can't load anything.`;
  if (/[;{}<>\\]|\/\*|var\s*\(|expression\s*\(|@import|javascript:/i.test(v))
    return `"${name}" has CSS in it. Values are plain colours or pixel sizes.`;
  if (v.length > VALUE_MAX) return `The value of "${name}" is too long.`;
  if (kind === "colour" && !isColourValue(v)) return `"${name}" needs a colour like #1A1A1A or rgb(26 26 26).`;
  if (kind === "length" && !isLengthValue(v)) return `"${name}" needs a size in pixels like 8px.`;
  return null;
}

/** Strip an optional leading "--" from a token name. */
const bare = (k: string) => k.replace(/^--/, "");

/** Check a theme file's parsed JSON. */
export function validateTheme(file: string, data: unknown): ThemeCheck {
  if (!data || typeof data !== "object" || Array.isArray(data)) return { ok: false, reason: "It isn't a theme: expected an object with name, base and tokens." };
  const o = data as Record<string, unknown>;
  for (const k of Object.keys(o))
    if (!FIELDS.includes(k)) return { ok: false, reason: `Unknown field "${k}". Theme files have name, base and tokens.` };
  if (typeof o.name !== "string" || !o.name.trim()) return { ok: false, reason: "It needs a name." };
  const name = o.name.trim();
  if (name.length > THEME_NAME_MAX || /[\u0000-\u001f\u007f<>]/.test(name)) return { ok: false, reason: `The name can be up to ${THEME_NAME_MAX} plain characters.` };
  if (o.author !== undefined && typeof o.author !== "string") return { ok: false, reason: "The author isn't text." };
  if (o.base !== "dark" && o.base !== "light") return { ok: false, reason: 'The base must be "dark" or "light".' };
  if (!o.tokens || typeof o.tokens !== "object" || Array.isArray(o.tokens)) return { ok: false, reason: "It needs a tokens object." };
  const tokens: Record<string, string> = {};
  const entries = Object.entries(o.tokens as Record<string, unknown>);
  if (entries.length > THEME_TOKEN_NAMES.length) return { ok: false, reason: "It has more tokens than Platter knows." };
  for (const [k, v] of entries) {
    const n = bare(k);
    if (!(n in THEME_TOKENS)) return { ok: false, reason: `Unknown token "${k}".` };
    const problem = tokenValueProblem(n, v);
    if (problem) return { ok: false, reason: problem };
    tokens[n] = (v as string).trim();
  }
  return { ok: true, theme: { file, name, base: o.base, tokens } };
}

/** Check a theme file's text. */
export function validateThemeText(file: string, text: string): ThemeCheck {
  if (new TextEncoder().encode(text).length > THEME_FILE_MAX_BYTES) return { ok: false, reason: "The file is too big for a theme (64 KB at most)." };
  let data: unknown;
  try {
    data = JSON.parse(text);
  } catch {
    return { ok: false, reason: "It isn't valid JSON." };
  }
  return validateTheme(file, data);
}
