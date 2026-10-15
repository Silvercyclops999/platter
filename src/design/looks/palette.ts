/**
 * Palettes for the look builder: a whole token set from one colour (an
 * album cover's, or a seeded random one). Surfaces take the colour's hue at
 * low saturation, the accent keeps it vivid, and every text and control
 * colour is fixed up to pass its contrast rule.
 */
import { hslToRgb, parseColour, rgbToHsl, type RGB } from "../../lib/color";
import { accentTokens, toHex } from "../themes/accent";
import { baseTheme } from "../themes/builtins";
import type { ThemeBase } from "../themes/validate";
import { CONTRAST_RULES, fixColour } from "./contrast";

const hsl = (h: number, s: number, l: number) => toHex(hslToRgb(((h % 360) + 360) % 360, Math.max(0, Math.min(1, s)), Math.max(0, Math.min(1, l))));

function rgba([r, g, b]: RGB, a: number): string {
  return `rgba(${Math.round(r)}, ${Math.round(g)}, ${Math.round(b)}, ${a})`;
}

/** A full token set built around one colour. */
export function paletteFrom(colour: string, base: ThemeBase): Record<string, string> {
  const rgb = parseColour(colour) ?? [128, 128, 128];
  const [h, s0] = rgbToHsl(rgb);
  const s = Math.min(s0, 0.85);
  const dark = base === "dark";
  const surface = dark ? Math.min(s * 0.35, 0.22) : Math.min(s * 0.3, 0.2);
  const t: Record<string, string> = { ...baseTheme(base).tokens };
  if (dark) {
    Object.assign(t, {
      "color-bg-base": hsl(h, surface, 0.045),
      "color-bg-raised": hsl(h, surface, 0.075),
      "color-bg-raised2": hsl(h, surface, 0.11),
      "color-bg-overlay": hsl(h, surface, 0.15),
      "color-border": hsl(h, surface, 0.18),
      "color-ink": hsl(h, s * 0.4, 0.92),
      "color-text-primary": hsl(h, s * 0.15, 0.95),
      "color-text-secondary": hsl(h, s * 0.15, 0.72),
      "color-text-muted": hsl(h, s * 0.12, 0.6),
    });
  } else {
    Object.assign(t, {
      "color-bg-base": hsl(h, surface, 0.88),
      "color-bg-raised": hsl(h, surface, 0.96),
      "color-bg-raised2": hsl(h, surface, 0.92),
      "color-bg-overlay": hsl(h, surface * 0.6, 0.99),
      "color-border": hsl(h, surface, 0.8),
      "color-ink": hsl(h, s * 0.4, 0.15),
      "color-text-primary": hsl(h, s * 0.2, 0.1),
      "color-text-secondary": hsl(h, s * 0.18, 0.3),
      "color-text-muted": hsl(h, s * 0.15, 0.38),
    });
  }
  const ink = parseColour(t["color-ink"]) ?? [255, 255, 255];
  t["color-state-hover"] = rgba(ink, 0.06);
  t["color-state-pressed"] = rgba(ink, 0.1);
  t["color-state-selected"] = rgba(ink, 0.12);
  const accent = hsl(h, Math.max(s, 0.45), dark ? 0.72 : 0.36);
  Object.assign(t, accentTokens(accent, base, t));
  return fixAll(t);
}

/** Every contrast rule fixed up, in rule order (the accent before text on it). */
export function fixAll(tokens: Record<string, string>): Record<string, string> {
  const t = { ...tokens };
  const order = [...CONTRAST_RULES].sort((a, b) => (a.token === "color-text-on-accent" ? 1 : b.token === "color-text-on-accent" ? -1 : 0));
  for (const rule of order) {
    const v = t[rule.token];
    if (!v) continue;
    const fixed = fixColour(v, rule.against, t, rule.min);
    if (fixed) t[rule.token] = fixed;
  }
  return t;
}

/** A small seeded random number generator (the same seed gives the same palette). */
export function seeded(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let x = a;
    x = Math.imul(x ^ (x >>> 15), x | 1);
    x ^= x + Math.imul(x ^ (x >>> 7), x | 61);
    return ((x ^ (x >>> 14)) >>> 0) / 4294967296;
  };
}

/** A palette from a seed: a random hue and saturation, the same every time for that seed. */
export function paletteFromSeed(seed: number, base: ThemeBase): Record<string, string> {
  const r = seeded(seed);
  const colour = hsl(r() * 360, 0.35 + r() * 0.5, 0.5);
  return paletteFrom(colour, base);
}
