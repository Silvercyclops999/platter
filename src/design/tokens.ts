/**
 * Design tokens for Platter.
 *
 * This file is the source of truth for values used from TypeScript (canvas
 * work, inline styles, tests). `tokens.css` declares the same values as CSS
 * custom properties; `tokens.test.ts` checks the two never drift apart.
 *
 * Identity: warm near-black surfaces (the window background #0E0D0C is set in
 * tauri.conf.json) and a single ivory accent, so album art is the only strong colour.
 */

/** Ivory ramp. 300 is the brand accent (a cream record mat on a black platter). */
export const ivory = {
  50: "#FFFDF8",
  100: "#F8F4EA",
  200: "#F2ECDF",
  300: "#ECE4D3",
  400: "#D6CCB8",
  500: "#B9AE98",
  600: "#8F8676",
  700: "#5E574D",
  800: "#3B3631",
  900: "#23201D",
} as const;

export const color = {
  // Background layers (warm near-black → lighter)
  bgBase: "#0E0D0C",
  bgRaised: "#171513",
  bgRaised2: "#201D1A",
  bgOverlay: "#2A2622",
  border: "#2F2A25",
  /** Base for faint lines and fills drawn over surfaces (mixed with transparent). */
  ink: "#FFF0DC",

  // Text
  textPrimary: "#F4EFE8",
  textSecondary: "#B8AFA4",
  textMuted: "#8A8178",
  textOnAccent: "#121110",

  // Accent + interaction states
  accent: ivory[300],
  accentHover: ivory[50],
  accentPressed: ivory[400],
  accentStrong: ivory[700],
  focusRing: ivory[50],
  stateHover: "rgba(255, 240, 220, 0.06)",
  statePressed: "rgba(255, 240, 220, 0.10)",
  stateSelected: "rgba(255, 240, 220, 0.12)",

  // The playing track (titles in lists, the bars marker, the playing card).
  // A soft warm coral: at least 5:1 on every surface and selected row, and
  // clearly apart from the ivory accent, success green and warning gold.
  playing: "#F0957A",

  // Semantic
  success: "#4CC38A",
  warning: "#F2B13D",
  warningText: "#FFEBC2",
  error: "#F2555A",
  errorText: "#FBD9DA",
  errorSurface: "#3A1A1B",
  info: "#5AA9F5",
  infoText: "#CFE5FB",
  /** Version badges (demo, live, remix). */
  version: "#9CCBF8",

  // Connection status dot
  statusLocal: "#3FCF8E",
  statusRemote: "#4C9EFF",
  statusRelay: "#F2B13D",
  statusOffline: "#F2555A",
} as const;

export const font = {
  family:
    '"Inter Variable", "Inter", system-ui, -apple-system, "Segoe UI", sans-serif',
} as const;

/** Type scale in px. */
export const fontSize = {
  xs: 12,
  sm: 13,
  md: 14,
  lg: 16,
  xl: 20,
  "2xl": 24,
  "3xl": 32,
  "4xl": 48,
} as const;

export const fontWeight = {
  regular: 400,
  semibold: 600,
  bold: 700,
} as const;

/** 4px grid. */
export const space = {
  1: 4,
  2: 8,
  3: 12,
  4: 16,
  6: 24,
  8: 32,
  12: 48,
  16: 64,
} as const;

export const radius = {
  row: 4,
  card: 8,
  pill: 999,
} as const;

/** Page-header title steps in px, largest first (each about two thirds of the one above). */
export const displaySize = [96, 64, 44, 32, 24] as const;

/** Extra-bold weight for page-header titles. */
export const displayWeight = 800;

/**
 * Motion bands in ms. Most changes are instant or quick; entrances animate,
 * exits are mostly instant. Reduced motion keeps these durations for fades
 * and colour, and drops movement (see tokens.css).
 */
export const motion = {
  instant: 0, // menus, row hover, panel close
  veryQuick: 70, // hover in (icon buttons)
  quick: 180, // card hover, image load, tooltips, panels in, hover out
  medium: 300, // queue rows
  slow: 500, // sticky title fade
  long: 900, // header colour
  /** Older name for `quick`, still used by some rules. */
  fast: 180,
  easing: "cubic-bezier(0.2, 0, 0, 1)", // decelerate
  easingInOut: "cubic-bezier(0.45, 0, 0.25, 1)",
} as const;

/**
 * Stacking order for everything that floats. Scroll thumbs sit under the top
 * bar (whose app and search menus hang over the sidebar) and so under every
 * menu, dialog, toast and tooltip. A scroller inside one of those layers gets
 * its thumb one step above that layer (src/design/scrollbars.ts).
 */
export const layer = {
  scrollbar: 590,
  topbar: 600,
  popover: 700,
  fullWindow: 750,
  fullWindowBars: 760,
  dialog: 800,
  menu: 900,
  toast: 950,
  tooltip: 1000,
} as const;

/** Map from token group/key to the CSS custom property that carries it. */
export function cssVar(name: string): string {
  return `var(--${name})`;
}

/** Flat list of CSS variable names → expected values, used by tests. */
export function cssVariableTable(): Record<string, string> {
  const t: Record<string, string> = {};
  const kebab = (s: string) => s.replace(/[A-Z]/g, (m) => "-" + m.toLowerCase());
  for (const [k, v] of Object.entries(ivory)) t[`ivory-${k}`] = v;
  for (const [k, v] of Object.entries(color)) t[`color-${kebab(k)}`] = v;
  for (const [k, v] of Object.entries(fontSize)) t[`font-size-${k}`] = `${v}px`;
  for (const [k, v] of Object.entries(fontWeight)) t[`font-weight-${k}`] = String(v);
  for (const [k, v] of Object.entries(space)) t[`space-${k}`] = `${v}px`;
  for (const [k, v] of Object.entries(radius)) t[`radius-${k}`] = `${v}px`;
  for (const [k, v] of Object.entries(motion)) t[`motion-${kebab(k)}`] = typeof v === "number" ? `${v}ms` : v;
  for (const [k, v] of Object.entries(layer)) t[`z-${kebab(k)}`] = String(v);
  displaySize.forEach((v, i) => (t[`font-size-display-${i + 1}`] = `${v}px`));
  t["font-weight-display"] = String(displayWeight);
  return t;
}
