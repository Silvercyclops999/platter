/**
 * Design tokens for Platter (spec 01).
 *
 * This file is the source of truth for values used from TypeScript (canvas
 * work, inline styles, tests). `tokens.css` declares the same values as CSS
 * custom properties; `tokens.test.ts` checks the two never drift apart.
 *
 * Identity: warm near-black surfaces (the window background #0E0D0C is set in
 * tauri.conf.json) and a single amber accent.
 */

/** Amber ramp. 400 is the brand accent. */
export const amber = {
  50: "#FFF8EB",
  100: "#FFEBC2",
  200: "#FFD98A",
  300: "#FCC459",
  400: "#F5A524",
  500: "#E08E0B",
  600: "#BA7208",
  700: "#8F560A",
  800: "#66400D",
  900: "#3D280C",
} as const;

export const color = {
  // Background layers (warm near-black → lighter)
  bgBase: "#0E0D0C",
  bgRaised: "#171513",
  bgRaised2: "#201D1A",
  bgOverlay: "#2A2622",
  border: "#2F2A25",

  // Text
  textPrimary: "#F4EFE8",
  textSecondary: "#B8AFA4",
  textMuted: "#8A8178",
  textOnAccent: "#1A1206",

  // Accent + interaction states
  accent: amber[400],
  accentHover: amber[300],
  accentPressed: amber[500],
  focusRing: amber[200],
  stateHover: "rgba(255, 240, 220, 0.06)",
  statePressed: "rgba(255, 240, 220, 0.10)",
  stateSelected: "rgba(255, 240, 220, 0.12)",

  // Semantic
  success: "#4CC38A",
  warning: "#F2B13D",
  error: "#F2555A",
  info: "#5AA9F5",

  // Connection status dot (spec 07)
  statusLocal: "#3FCF8E",
  statusRemote: "#4C9EFF",
  statusRelay: "#F5A524",
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

/** Durations in ms. All become 0 under reduced motion. */
export const motion = {
  fast: 150, // hover
  medium: 250, // panels
  slow: 350, // page transitions
  easing: "cubic-bezier(0.2, 0, 0, 1)", // standard ease-out
} as const;

/** Map from token group/key to the CSS custom property that carries it. */
export function cssVar(name: string): string {
  return `var(--${name})`;
}

/** Flat list of CSS variable names → expected values, used by tests. */
export function cssVariableTable(): Record<string, string> {
  const t: Record<string, string> = {};
  const kebab = (s: string) => s.replace(/[A-Z]/g, (m) => "-" + m.toLowerCase());
  for (const [k, v] of Object.entries(amber)) t[`amber-${k}`] = v;
  for (const [k, v] of Object.entries(color)) t[`color-${kebab(k)}`] = v;
  for (const [k, v] of Object.entries(fontSize)) t[`font-size-${k}`] = `${v}px`;
  for (const [k, v] of Object.entries(fontWeight)) t[`font-weight-${k}`] = String(v);
  for (const [k, v] of Object.entries(space)) t[`space-${k}`] = `${v}px`;
  for (const [k, v] of Object.entries(radius)) t[`radius-${k}`] = `${v}px`;
  t["motion-fast"] = `${motion.fast}ms`;
  t["motion-medium"] = `${motion.medium}ms`;
  t["motion-slow"] = `${motion.slow}ms`;
  t["motion-easing"] = motion.easing;
  return t;
}
