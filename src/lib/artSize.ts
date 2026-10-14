/**
 * Artwork request sizes.
 *
 * Art is requested at the rendered CSS size times the display's pixel ratio,
 * rounded up to a fixed ladder of sizes so nearby sizes share one cache entry
 * (in the core's disk cache and the WebView's memory cache). Decoded images
 * cost width × height × 4 bytes, so asking for 2× on a 1× display costs four
 * times the memory for no visible gain.
 */

/** Pixel sizes art is ever requested at (steps of roughly 1.25×, with an
 * extra 280 so 1.5× displays don't round card art up to 320). */
export const ART_STEPS = [32, 40, 48, 64, 80, 96, 128, 160, 192, 240, 280, 320, 400, 480, 640, 800, 960, 1280] as const;

const MAX_PX = 4096;

/** The display's pixel ratio, clamped to a sane range. */
export function devicePixelRatio(win: { devicePixelRatio?: number } | undefined = typeof window !== "undefined" ? window : undefined): number {
  const r = win?.devicePixelRatio;
  return typeof r === "number" && Number.isFinite(r) && r > 0 ? Math.min(Math.max(r, 1), 4) : 1;
}

/** Smallest ladder step that covers `px` device pixels. */
export function artBucket(px: number): number {
  const need = Math.max(1, Math.ceil(px));
  for (const s of ART_STEPS) if (s >= need) return s;
  const top = ART_STEPS[ART_STEPS.length - 1];
  return Math.min(MAX_PX, Math.ceil(need / top) * top);
}

/** Pixel size to request for art shown at `css` CSS px. */
export function artPixelSize(css: number, dpr: number = devicePixelRatio()): number {
  return artBucket(css * dpr);
}
