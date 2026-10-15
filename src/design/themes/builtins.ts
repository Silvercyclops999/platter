/**
 * Built-in themes. Each is only a set of token values (the names in
 * contract.json); nothing else about the app changes between them. Dark is
 * the values in tokens.ts / tokens.css, so it needs no overrides.
 */
import { color, radius } from "../tokens";
import { THEME_TOKEN_NAMES, type ThemeBase } from "./validate";

export type BuiltinThemeId = "dark" | "light" | "midnight" | "forest" | "sand";

export interface BuiltinTheme {
  id: BuiltinThemeId;
  name: string;
  base: ThemeBase;
  tokens: Record<string, string>;
}

const kebab = (s: string) => s.replace(/[A-Z]/g, (m) => "-" + m.toLowerCase());

/** The Dark theme's tokens, straight from tokens.ts. */
function darkTokens(): Record<string, string> {
  const t: Record<string, string> = {};
  for (const [k, v] of Object.entries(color)) {
    const name = `color-${kebab(k)}`;
    if (THEME_TOKEN_NAMES.includes(name)) t[name] = v;
  }
  for (const [k, v] of Object.entries(radius)) t[`radius-${k}`] = `${v}px`;
  return t;
}

const DARK = darkTokens();
const SHAPE = { "radius-row": DARK["radius-row"], "radius-card": DARK["radius-card"], "radius-pill": DARK["radius-pill"] };

/** Status, semantic and playing colours shared by the dark themes. */
const DARK_SIGNALS = {
  "color-playing": DARK["color-playing"],
  "color-success": DARK["color-success"],
  "color-warning": DARK["color-warning"],
  "color-warning-text": DARK["color-warning-text"],
  "color-error": DARK["color-error"],
  "color-error-text": DARK["color-error-text"],
  "color-info": DARK["color-info"],
  "color-info-text": DARK["color-info-text"],
  "color-version": DARK["color-version"],
  "color-status-local": DARK["color-status-local"],
  "color-status-remote": DARK["color-status-remote"],
  "color-status-relay": DARK["color-status-relay"],
  "color-status-offline": DARK["color-status-offline"],
};

/** The ivory accent on a dark surface. */
const IVORY_ON_DARK = {
  "color-accent": DARK["color-accent"],
  "color-accent-hover": DARK["color-accent-hover"],
  "color-accent-pressed": DARK["color-accent-pressed"],
  "color-focus-ring": DARK["color-focus-ring"],
};

/** Status, semantic and playing colours shared by the light themes, dark enough for light surfaces. */
const LIGHT_SIGNALS = {
  "color-success": "#1D7A4D",
  "color-warning": "#8F5D00",
  "color-warning-text": "#5C3C00",
  "color-error": "#A3272A",
  "color-error-text": "#86191D",
  "color-error-surface": "#F6DCDA",
  "color-info": "#1C66B0",
  "color-info-text": "#14477D",
  "color-version": "#1B5A94",
  "color-status-local": "#1E9C63",
  "color-status-remote": "#2F78D6",
  "color-status-relay": "#C88A12",
  "color-status-offline": "#D63A3F",
};

/** The ivory accent on a light surface: the deep end of the ivory ramp. */
const IVORY_ON_LIGHT = {
  "color-accent": "#5E574D",
  "color-accent-hover": "#4A443C",
  "color-accent-pressed": "#3B3631",
  "color-focus-ring": "#3B3631",
  "color-text-on-accent": "#FFFDF8",
};

export const BUILTIN_THEMES: Record<BuiltinThemeId, BuiltinTheme> = {
  dark: { id: "dark", name: "Dark", base: "dark", tokens: DARK },
  light: {
    id: "light",
    name: "Light",
    base: "light",
    tokens: {
      "color-bg-base": "#E4DED3",
      "color-bg-raised": "#F7F4EE",
      "color-bg-raised2": "#EDE8DF",
      "color-bg-overlay": "#FFFFFF",
      "color-border": "#D5CDBF",
      "color-ink": "#2A2118",
      "color-text-primary": "#1B1814",
      "color-text-secondary": "#51483E",
      "color-text-muted": "#655C52",
      ...IVORY_ON_LIGHT,
      "color-accent-strong": "#B9AE98",
      "color-state-hover": "rgba(42, 33, 24, 0.06)",
      "color-state-pressed": "rgba(42, 33, 24, 0.10)",
      "color-state-selected": "rgba(42, 33, 24, 0.09)",
      "color-playing": "#A2391D",
      ...LIGHT_SIGNALS,
      ...SHAPE,
    },
  },
  midnight: {
    id: "midnight",
    name: "Midnight",
    base: "dark",
    tokens: {
      "color-bg-base": "#0A0D13",
      "color-bg-raised": "#111620",
      "color-bg-raised2": "#18202D",
      "color-bg-overlay": "#212B3B",
      "color-border": "#253043",
      "color-ink": "#DCE6FF",
      "color-text-primary": "#EEF2FA",
      "color-text-secondary": "#AAB4C6",
      "color-text-muted": "#8792A7",
      "color-text-on-accent": "#10131A",
      ...IVORY_ON_DARK,
      "color-accent-strong": "#4A5468",
      "color-state-hover": "rgba(220, 230, 255, 0.06)",
      "color-state-pressed": "rgba(220, 230, 255, 0.10)",
      "color-state-selected": "rgba(220, 230, 255, 0.12)",
      ...DARK_SIGNALS,
      "color-error-surface": "#3A1A22",
      ...SHAPE,
    },
  },
  forest: {
    id: "forest",
    name: "Forest",
    base: "dark",
    tokens: {
      "color-bg-base": "#0B100D",
      "color-bg-raised": "#121914",
      "color-bg-raised2": "#19221C",
      "color-bg-overlay": "#222D25",
      "color-border": "#26332A",
      "color-ink": "#E2F2E6",
      "color-text-primary": "#EEF4EF",
      "color-text-secondary": "#ABBAAE",
      "color-text-muted": "#869689",
      "color-text-on-accent": "#111410",
      ...IVORY_ON_DARK,
      "color-accent-strong": "#4C5A4F",
      "color-state-hover": "rgba(226, 242, 230, 0.06)",
      "color-state-pressed": "rgba(226, 242, 230, 0.10)",
      "color-state-selected": "rgba(226, 242, 230, 0.12)",
      ...DARK_SIGNALS,
      "color-error-surface": "#3A1A1B",
      ...SHAPE,
    },
  },
  sand: {
    id: "sand",
    name: "Sand",
    base: "light",
    tokens: {
      "color-bg-base": "#E5D9C6",
      "color-bg-raised": "#F4ECDF",
      "color-bg-raised2": "#EADFCD",
      "color-bg-overlay": "#FBF7F0",
      "color-border": "#D0C1A9",
      "color-ink": "#3A2A18",
      "color-text-primary": "#22190F",
      "color-text-secondary": "#54452F",
      "color-text-muted": "#67573F",
      ...IVORY_ON_LIGHT,
      "color-accent-strong": "#B9A585",
      "color-state-hover": "rgba(58, 42, 24, 0.06)",
      "color-state-pressed": "rgba(58, 42, 24, 0.10)",
      "color-state-selected": "rgba(58, 42, 24, 0.10)",
      "color-playing": "#97351A",
      ...LIGHT_SIGNALS,
      ...SHAPE,
    },
  },
};

export const BUILTIN_IDS = Object.keys(BUILTIN_THEMES) as BuiltinThemeId[];

/** The built-in a theme file starts from. */
export function baseTheme(base: ThemeBase): BuiltinTheme {
  return base === "light" ? BUILTIN_THEMES.light : BUILTIN_THEMES.dark;
}
