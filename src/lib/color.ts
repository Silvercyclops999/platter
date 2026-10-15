/**
 * Page header colours from artwork. The header takes the art's most vivid
 * colour (not its average): pixels are grouped into hue buckets weighted by
 * saturation x chroma, and the strongest bucket's mean wins. A small bright
 * detail on a pale cover sets the whole header. Greyscale art gets a neutral
 * grey. From that colour come a vivid tone and a dark shade, both kept dark
 * enough for the header's light text.
 */
import { color } from "../design/tokens";
import { setBounded } from "./bounded";

export type RGB = [number, number, number];

/** The two header colours: the vivid tone (top) and the dark shade band. */
export interface HeaderColors {
  vivid: string;
  shade: string;
}

/** Header colours for pages without art: the page's own surfaces. */
export const NEUTRAL_HEADER: HeaderColors = { vivid: "#2a2520", shade: color.bgRaised };
/** The same on other themes: the theme's own panel colours. */
export const THEMED_NEUTRAL_HEADER: Record<"dark" | "light", HeaderColors> = {
  dark: { vivid: "var(--color-bg-overlay)", shade: "var(--color-bg-raised)" },
  light: { vivid: "var(--color-bg-raised2)", shade: "var(--color-bg-raised)" },
};

export function hexToRgb(hex: string): RGB | null {
  const h = hex.replace(/^#/, "");
  if (!/^[0-9a-f]{6}$/i.test(h)) return null;
  return [parseInt(h.slice(0, 2), 16), parseInt(h.slice(2, 4), 16), parseInt(h.slice(4, 6), 16)];
}

/** Parse #rgb, #rrggbb, #rrggbbaa or rgb()/rgba() into RGB (alpha ignored). */
export function parseColour(v: string): RGB | null {
  const s = v.trim();
  let m = /^#([0-9a-f]{3,4})$/i.exec(s);
  if (m) {
    const h = m[1];
    return [0, 1, 2].map((i) => parseInt(h[i] + h[i], 16)) as RGB;
  }
  m = /^#([0-9a-f]{6})(?:[0-9a-f]{2})?$/i.exec(s);
  if (m) {
    const h = m[1];
    return [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16)) as RGB;
  }
  m = /^rgba?\(\s*([\d.]+)\s*[,\s]\s*([\d.]+)\s*[,\s]\s*([\d.]+)/i.exec(s);
  if (m) return [Number(m[1]), Number(m[2]), Number(m[3])];
  return null;
}

export function rgbToCss([r, g, b]: RGB): string {
  return `rgb(${Math.round(r)} ${Math.round(g)} ${Math.round(b)})`;
}

export function rgbToHsl([r, g, b]: RGB): [number, number, number] {
  r /= 255;
  g /= 255;
  b /= 255;
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const l = (max + min) / 2;
  if (max === min) return [0, 0, l];
  const d = max - min;
  const s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
  let h = 0;
  if (max === r) h = (g - b) / d + (g < b ? 6 : 0);
  else if (max === g) h = (b - r) / d + 2;
  else h = (r - g) / d + 4;
  return [h * 60, s, l];
}

export function hslToRgb(h: number, s: number, l: number): RGB {
  const k = (n: number) => (n + h / 30) % 12;
  const a = s * Math.min(l, 1 - l);
  const f = (n: number) => l - a * Math.max(-1, Math.min(k(n) - 3, Math.min(9 - k(n), 1)));
  return [f(0) * 255, f(8) * 255, f(4) * 255];
}

/** WCAG relative luminance. */
export function luminance([r, g, b]: RGB): number {
  const lin = (c: number) => {
    const v = c / 255;
    return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
}

/** WCAG contrast ratio between two colours (1 to 21). */
export function contrastRatio(a: RGB, b: RGB): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

/** Header text colour, which every header colour must stay readable under. */
export const HEADER_TEXT: RGB = hexToRgb(color.textPrimary)!;

const HUE_BUCKETS = 18; // 20 degrees each
/** Below this chroma a pixel counts as grey (paper, black, white, JPEG noise). */
const MIN_CHROMA = 0.1;

/**
 * Most vivid colour of an RGBA buffer, or null when the art is effectively
 * greyscale. Each coloured pixel adds saturation x chroma to its hue bucket
 * (half of that to the two neighbouring buckets, so a hue split across a
 * bucket edge still counts as one); the strongest bucket's weighted mean is
 * the result.
 */
export function vividFromPixels(data: Uint8ClampedArray | number[]): RGB | null {
  const weight = new Float64Array(HUE_BUCKETS);
  const sum = Array.from({ length: HUE_BUCKETS }, () => [0, 0, 0]);
  let pixels = 0;
  for (let i = 0; i < data.length; i += 4) {
    if (data[i + 3] < 128) continue;
    pixels++;
    const rgb: RGB = [data[i], data[i + 1], data[i + 2]];
    const chroma = (Math.max(...rgb) - Math.min(...rgb)) / 255;
    if (chroma < MIN_CHROMA) continue;
    const [h, s] = rgbToHsl(rgb);
    const w = s * chroma;
    const b = Math.floor(h / (360 / HUE_BUCKETS)) % HUE_BUCKETS;
    weight[b] += w;
    sum[b][0] += rgb[0] * w;
    sum[b][1] += rgb[1] * w;
    sum[b][2] += rgb[2] * w;
  }
  let best = -1;
  let bestScore = 0;
  for (let b = 0; b < HUE_BUCKETS; b++) {
    const score = weight[b] + 0.5 * (weight[(b + HUE_BUCKETS - 1) % HUE_BUCKETS] + weight[(b + 1) % HUE_BUCKETS]);
    if (score > bestScore) {
      bestScore = score;
      best = b;
    }
  }
  // A few stray coloured pixels in a grey cover don't make it colourful.
  if (best < 0 || weight[best] < Math.max(0.5, pixels * 0.004)) return null;
  const w = weight[best];
  return [sum[best][0] / w, sum[best][1] / w, sum[best][2] / w];
}

/** Lightness of the dark shade band: well under half brightness. */
const SHADE_L = 0.16;
/** Minimum contrast of header text over the vivid tone (large-text AA plus margin). */
const VIVID_MIN_CONTRAST = 3.2;

/**
 * Vivid tone and dark shade for a picked colour; null (greyscale art) gives
 * a neutral grey. The vivid tone keeps its hue and stays saturated, darkened
 * only as far as header text contrast needs.
 */
export function headerColors(rgb: RGB | null): HeaderColors {
  if (!rgb) return { vivid: "rgb(83 83 83)", shade: "rgb(36 36 36)" };
  const [h, s0, l0] = rgbToHsl(rgb);
  const s = Math.min(0.9, Math.max(0.5, s0));
  let l = Math.min(0.5, Math.max(0.3, l0));
  let vivid = hslToRgb(h, s, l);
  while (contrastRatio(HEADER_TEXT, vivid) < VIVID_MIN_CONTRAST && l > 0.05) {
    l -= 0.01;
    vivid = hslToRgb(h, s, l);
  }
  const shade = hslToRgb(h, Math.min(s, 0.7), SHADE_L);
  return { vivid: rgbToCss(vivid), shade: rgbToCss(shade) };
}

/** Header text on light themes (the Light theme's primary text). */
export const LIGHT_HEADER_TEXT: RGB = [27, 24, 20];
/** Lightness of the pale band on light themes. */
const LIGHT_SHADE_L = 0.87;
/** Dark header text over the vivid tone on light themes: full AA. */
const LIGHT_VIVID_MIN_CONTRAST = 4.5;

/**
 * Header colours for a light theme, from the dark-theme pair: the same hue
 * as a pale band with a lighter vivid tone above it, both light enough for
 * the dark header text. Greyscale art stays a warm grey.
 */
export function lightHeader(c: HeaderColors): HeaderColors {
  const v = parseColour(c.vivid) ?? [83, 83, 83];
  const [h, s0] = rgbToHsl(v);
  const grey = s0 < 0.08;
  const s = grey ? 0.08 : Math.min(0.6, Math.max(0.35, s0));
  let l = 0.66;
  let vivid = hslToRgb(grey ? 36 : h, s, l);
  while (contrastRatio(LIGHT_HEADER_TEXT, vivid) < LIGHT_VIVID_MIN_CONTRAST && l < 0.95) {
    l += 0.01;
    vivid = hslToRgb(grey ? 36 : h, s, l);
  }
  const shade = hslToRgb(grey ? 36 : h, grey ? 0.1 : Math.min(s, 0.45), LIGHT_SHADE_L);
  return { vivid: rgbToCss(vivid), shade: rgbToCss(shade) };
}

/** Header colours for the theme in use: as picked on dark themes, pale on light ones. */
export function themedHeader(c: HeaderColors, base: "dark" | "light"): HeaderColors {
  return base === "light" ? lightHeader(c) : c;
}

/** Header colours from a server-computed colour (hex), used when the art can't be sampled. */
export function headerColorsFromServer(tint: string | null | undefined): HeaderColors | null {
  if (!tint) return null;
  const rgb = hexToRgb(tint);
  if (!rgb) return null;
  const chroma = (Math.max(...rgb) - Math.min(...rgb)) / 255;
  return headerColors(chroma < MIN_CHROMA ? null : rgb);
}

/** Side of the square the art is scaled down to before sampling. */
const SAMPLE = 24;

/** Sample a loaded <img>. Returns undefined if the canvas is unavailable or tainted. */
export function vividFromImage(img: HTMLImageElement): RGB | null | undefined {
  try {
    const c = document.createElement("canvas");
    c.width = SAMPLE;
    c.height = SAMPLE;
    const ctx = c.getContext("2d", { willReadFrequently: true });
    if (!ctx) return undefined;
    ctx.drawImage(img, 0, 0, SAMPLE, SAMPLE);
    return vividFromPixels(ctx.getImageData(0, 0, SAMPLE, SAMPLE).data);
  } catch {
    return undefined;
  }
}

const imageColorCache = new Map<string, HeaderColors>();

/** Header colours for an artwork URL, or null when the image can't be read. */
export function headerColorsFromImageUrl(url: string): Promise<HeaderColors | null> {
  const hit = imageColorCache.get(url);
  if (hit) return Promise.resolve(hit);
  return new Promise((resolve) => {
    const img = new Image();
    img.crossOrigin = "anonymous";
    img.decoding = "async";
    img.onload = () => {
      const picked = vividFromImage(img);
      if (picked === undefined) return resolve(null);
      const colors = headerColors(picked);
      setBounded(imageColorCache, url, colors, 300);
      resolve(colors);
    };
    img.onerror = () => resolve(null);
    img.src = url;
  });
}
