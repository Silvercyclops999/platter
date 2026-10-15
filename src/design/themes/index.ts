/**
 * Themes: pick the token set from the Theme, Accent colour and Theme file
 * settings, and put it on <html>. Dark's values live in tokens.css, so the
 * Dark theme with the ivory accent sets nothing inline; every other theme
 * sets the tokens that differ. data-theme-base tells CSS whether the
 * surfaces are light or dark.
 */
import { accentColour, accentProblem, accentTokens } from "./accent";
import { BUILTIN_THEMES, baseTheme, type BuiltinThemeId } from "./builtins";
import { THEME_TOKEN_NAMES, tokenValueProblem, type ThemeBase, type UserTheme } from "./validate";

export * from "./accent";
export * from "./builtins";
export * from "./validate";

/** The Theme setting: a built-in, or follow the computer's light or dark mode. */
export type ThemeSetting = BuiltinThemeId | "system";

export interface ThemeChoice {
  theme: ThemeSetting;
  accent: string;
  themeFile: UserTheme | null;
}

export interface ResolvedTheme {
  /** A built-in id, or "file" for a theme file. */
  id: BuiltinThemeId | "file";
  name: string;
  base: ThemeBase;
  /** Every theme token. */
  tokens: Record<string, string>;
  /** Set when the chosen accent couldn't be used on this theme. */
  accentNote: string | null;
  /** The theme file sets its own accent (used while the setting is Ivory). */
  fileAccent: boolean;
}

export const DEFAULT_CHOICE: ThemeChoice = { theme: "dark", accent: "ivory", themeFile: null };

/** Work out the tokens for a choice. `systemDark` is the computer's mode. */
export function resolveTheme(choice: ThemeChoice, systemDark: boolean): ResolvedTheme {
  let id: ResolvedTheme["id"];
  let name: string;
  let base: ThemeBase;
  let tokens: Record<string, string>;
  let fileAccent = false;
  if (choice.themeFile) {
    const f = choice.themeFile;
    const start = baseTheme(f.base);
    tokens = { ...start.tokens };
    // Checked again here: a file can arrive through the sync file.
    for (const [k, v] of Object.entries(f.tokens)) if (THEME_TOKEN_NAMES.includes(k) && !tokenValueProblem(k, v)) tokens[k] = v;
    fileAccent = "color-accent" in f.tokens;
    id = "file";
    name = f.name;
    base = f.base;
  } else {
    const builtin = choice.theme === "system" ? (systemDark ? BUILTIN_THEMES.dark : BUILTIN_THEMES.light) : (BUILTIN_THEMES[choice.theme] ?? BUILTIN_THEMES.dark);
    ({ id, name, base } = builtin);
    tokens = { ...builtin.tokens };
  }
  let accentNote: string | null = null;
  // Ivory is each theme's own accent (or the theme file's, when it sets one).
  const colour = choice.accent === "ivory" ? null : accentColour(choice.accent, base);
  if (colour) {
    if (accentProblem(colour, tokens)) accentNote = "Your colour is too close to this theme's background, so the theme's own accent is used.";
    else Object.assign(tokens, accentTokens(colour, base, tokens));
  }
  return { id, name, base, tokens, accentNote, fileAccent };
}

const DARK = BUILTIN_THEMES.dark.tokens;
const CACHE_KEY = "platter.theme";

/** Put a resolved theme on the document. */
export function applyTheme(t: Pick<ResolvedTheme, "id" | "base" | "tokens">, doc: Document = document): void {
  const root = doc.documentElement;
  root.dataset.theme = t.id;
  root.dataset.themeBase = t.base;
  const overrides: Record<string, string> = {};
  for (const name of THEME_TOKEN_NAMES) {
    const v = t.tokens[name];
    if (v && v.toLowerCase() !== DARK[name]?.toLowerCase()) {
      root.style.setProperty(`--${name}`, v);
      overrides[name] = v;
    } else root.style.removeProperty(`--${name}`);
  }
  try {
    localStorage.setItem(CACHE_KEY, JSON.stringify({ id: t.id, base: t.base, tokens: overrides }));
  } catch {
    // Storage can be unavailable; the theme still applies once settings load.
  }
}

/**
 * Apply the last theme used on this computer before the first paint, so a
 * light theme doesn't flash dark while settings load.
 */
export function applyCachedTheme(doc: Document = document): void {
  let raw: string | null = null;
  try {
    raw = localStorage.getItem(CACHE_KEY);
  } catch {
    return;
  }
  if (!raw) return;
  try {
    const c = JSON.parse(raw) as { id?: string; base?: string; tokens?: Record<string, unknown> };
    const tokens: Record<string, string> = { ...DARK };
    for (const [k, v] of Object.entries(c.tokens ?? {}))
      if (THEME_TOKEN_NAMES.includes(k) && typeof v === "string" && !tokenValueProblem(k, v)) tokens[k] = v;
    const base: ThemeBase = c.base === "light" ? "light" : "dark";
    const id = (c.id && (c.id in BUILTIN_THEMES || c.id === "file") ? c.id : "dark") as ResolvedTheme["id"];
    applyTheme({ id, base, tokens }, doc);
  } catch {
    // A broken cache entry is ignored.
  }
}
