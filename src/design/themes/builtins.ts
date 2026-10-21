/**
 * Built-in themes. Each is only a set of token values (the names in
 * contract.json); nothing else about the app changes between them. Classic
 * (id "dark") is the values in tokens.ts / tokens.css, so it needs no
 * overrides. Studio, Studio Black and Sleeve are Platter's own looks: they
 * share a brass accent, and CSS adds their surfaces (data-identity on <html>).
 */
import { color, radius } from "../tokens";
import { THEME_TOKEN_NAMES, type ThemeBase } from "./validate";

export type BuiltinThemeId = "dark" | "light" | "midnight" | "forest" | "sand" | "studio" | "studioBlack" | "sleeve";

export interface BuiltinTheme {
  id: BuiltinThemeId;
  name: string;
  base: ThemeBase;
  tokens: Record<string, string>;
  /** The accent preset that is this theme's own (no accent tokens are set for it). */
  ownAccent: "ivory" | "brass";
}

/** Platter's own looks, which share the brass accent and the record-player motif. */
export const STUDIO_IDS: readonly BuiltinThemeId[] = ["studio", "studioBlack", "sleeve"];

/** True for Studio, Studio Black and Sleeve. */
export function isStudioLook(id: string | null | undefined): boolean {
  return !!id && (STUDIO_IDS as readonly string[]).includes(id);
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

/** Brass on the Studio surfaces: the accent, the playing row and focus. */
const BRASS_ON_DARK = {
  "color-accent": "#CDAE6E",
  "color-accent-hover": "#DCC28A",
  "color-accent-pressed": "#B4955A",
  "color-focus-ring": "#E2CB95",
  "color-text-on-accent": "#1A1610",
};

/** Studio: warm graphite, like the front of rack gear. */
const STUDIO: Record<string, string> = {
  "color-bg-base": "#1A1816",
  "color-bg-raised": "#211F1C",
  "color-bg-raised2": "#2A2724",
  "color-bg-overlay": "#33302C",
  "color-border": "#3A3632",
  "color-ink": "#FFF3E0",
  "color-text-primary": "#F2EDE4",
  "color-text-secondary": "#BDB4A7",
  "color-text-muted": "#A39A8D",
  ...BRASS_ON_DARK,
  "color-accent-strong": "#6B5C3E",
  "color-state-hover": "rgba(255, 243, 224, 0.06)",
  "color-state-pressed": "rgba(255, 243, 224, 0.10)",
  "color-state-selected": "rgba(255, 243, 224, 0.12)",
  ...DARK_SIGNALS,
  "color-playing": "#CDAE6E",
  "color-error-surface": "#3A1D1C",
  "radius-row": "4px",
  "radius-card": "6px",
  "radius-pill": "999px",
};

/** Studio Black: the same, on true black for OLED screens. */
const STUDIO_BLACK: Record<string, string> = {
  ...STUDIO,
  "color-bg-base": "#000000",
  "color-bg-raised": "#0D0C0B",
  "color-bg-raised2": "#171614",
  "color-bg-overlay": "#201E1B",
  "color-border": "#2A2724",
  "color-accent-strong": "#5E5034",
  "color-error-surface": "#2E1514",
};

/** Sleeve: a clean warm paper and ink, with a darker brass that reads on paper. The phone and the computer share it. */
const SLEEVE: Record<string, string> = {
  "color-bg-base": "#F4F0E8",
  "color-bg-raised": "#FBF9F4",
  "color-bg-raised2": "#ECE6DA",
  "color-bg-overlay": "#FDFCF8",
  "color-border": "#DCD3C2",
  "color-ink": "#2B2117",
  "color-text-primary": "#1C1712",
  "color-text-secondary": "#4B4237",
  "color-text-muted": "#5E5546",
  "color-accent": "#7A5A1C",
  "color-accent-hover": "#664A15",
  "color-accent-pressed": "#563F12",
  "color-focus-ring": "#563F12",
  "color-text-on-accent": "#FFFBF2",
  "color-accent-strong": "#C9B58A",
  "color-state-hover": "rgba(43, 33, 23, 0.06)",
  "color-state-pressed": "rgba(43, 33, 23, 0.10)",
  "color-state-selected": "rgba(43, 33, 23, 0.09)",
  ...LIGHT_SIGNALS,
  "color-playing": "#7A5A1C",
  "radius-row": "4px",
  "radius-card": "6px",
  "radius-pill": "999px",
};

export const BUILTIN_THEMES: Record<BuiltinThemeId, BuiltinTheme> = {
  studio: { id: "studio", name: "Studio", base: "dark", tokens: STUDIO, ownAccent: "brass" },
  studioBlack: { id: "studioBlack", name: "Studio Black", base: "dark", tokens: STUDIO_BLACK, ownAccent: "brass" },
  sleeve: { id: "sleeve", name: "Sleeve", base: "light", tokens: SLEEVE, ownAccent: "brass" },
  dark: { id: "dark", name: "Classic", base: "dark", tokens: DARK, ownAccent: "ivory" },
  light: {
    id: "light",
    name: "Classic light",
    base: "light",
    ownAccent: "ivory",
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
    ownAccent: "ivory",
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
    ownAccent: "ivory",
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
    ownAccent: "ivory",
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

/**
 * The look shown for a choice while following the computer's light or dark
 * mode: the choice itself when its base matches, otherwise its partner.
 * Studio and Studio Black pair with Sleeve; the other dark themes pair with
 * Classic light, and the light ones with Classic.
 */
export function partnerFor(id: BuiltinThemeId, systemDark: boolean): BuiltinThemeId {
  const t = BUILTIN_THEMES[id];
  if ((t.base === "dark") === systemDark) return id;
  if (isStudioLook(id)) return systemDark ? "studio" : "sleeve";
  return systemDark ? "dark" : "light";
}
