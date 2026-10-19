/**
 * Cover light: the playing cover's colour used as light on the now playing
 * area of Platter's own looks, never as paint. The colour is clamped so a
 * neon or white cover can't flood the panel or wash out its text, and it is
 * only ever mixed in a little (instruments.css): a single radial pool and a
 * lit top edge, both static.
 */
import { hslToRgb, luminance, parseColour, rgbToCss, rgbToHsl, type RGB } from "../lib/color";
import type { CoverLight } from "../lib/types";

/** The most saturated the light gets. */
const MAX_SAT = 0.55;

/** How much of the light is mixed into the surface at the pool's centre, in percent. */
export const COVER_LIGHT_MIX: Record<"dark" | "light", Record<Exclude<CoverLight, "off">, number>> = {
  dark: { on: 16, less: 8 },
  light: { on: 12, less: 6 },
};

/**
 * The light for a cover colour: its hue, saturation capped, lightness held to
 * a band that reads as a glow on graphite (or a tint on paper). Grey and
 * black covers give a warm neutral glow rather than nothing.
 */
export function coverLightColour(colour: string, base: "dark" | "light"): string | null {
  const rgb = parseColour(colour);
  if (!rgb) return null;
  const [h, s0] = rgbToHsl(rgb);
  const s = Math.min(MAX_SAT, s0 < 0.08 ? 0.12 : s0);
  const hue = s0 < 0.08 ? 36 : h;
  let l = base === "dark" ? 0.56 : 0.5;
  let out = hslToRgb(hue, s, l);
  // Greens and yellows look brighter at the same lightness: hold the light's
  // brightness down so muted text over the pool keeps its contrast.
  while (base === "dark" && luminance(out) > MAX_LUMINANCE && l > 0.3) {
    l -= 0.02;
    out = hslToRgb(hue, s, l);
  }
  return rgbToCss(out);
}

/** The brightest the light gets on graphite (WCAG relative luminance). */
const MAX_LUMINANCE = 0.16;

/** `a` mixed `pct` percent into `b`, as srgb color-mix does. */
export function mixRgb(a: RGB, b: RGB, pct: number): RGB {
  const t = pct / 100;
  return [0, 1, 2].map((i) => Math.round(a[i] * t + b[i] * (1 - t))) as RGB;
}
