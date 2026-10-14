/**
 * Header tint helpers: use the server's tint when present, otherwise extract a
 * dominant color from the loaded artwork with a tiny canvas sample.
 */

export type RGB = [number, number, number];

export function hexToRgb(hex: string): RGB | null {
  const h = hex.replace(/^#/, "");
  if (!/^[0-9a-f]{6}$/i.test(h)) return null;
  return [parseInt(h.slice(0, 2), 16), parseInt(h.slice(2, 4), 16), parseInt(h.slice(4, 6), 16)];
}

export function rgbToCss([r, g, b]: RGB): string {
  return `rgb(${Math.round(r)} ${Math.round(g)} ${Math.round(b)})`;
}

function rgbToHsl([r, g, b]: RGB): [number, number, number] {
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

function hslToRgb(h: number, s: number, l: number): RGB {
  const k = (n: number) => (n + h / 30) % 12;
  const a = s * Math.min(l, 1 - l);
  const f = (n: number) => l - a * Math.max(-1, Math.min(k(n) - 3, Math.min(9 - k(n), 1)));
  return [f(0) * 255, f(8) * 255, f(4) * 255];
}

/** Clamp a color into a range that keeps white header text readable. */
export function tameTint(rgb: RGB): RGB {
  const [h, s, l] = rgbToHsl(rgb);
  return hslToRgb(h, Math.min(s, 0.6), Math.max(0.16, Math.min(l, 0.32)));
}

/** Average of the most saturated pixels of an RGBA buffer. */
export function dominantFromPixels(data: Uint8ClampedArray | number[]): RGB {
  const buckets: { rgb: RGB; weight: number }[] = [];
  for (let i = 0; i < data.length; i += 4) {
    if (data[i + 3] < 128) continue;
    const rgb: RGB = [data[i], data[i + 1], data[i + 2]];
    const [, s, l] = rgbToHsl(rgb);
    // Weight saturated mid-tones; ignore near black/white.
    const w = s * (1 - Math.abs(l - 0.5) * 1.6);
    if (w > 0) buckets.push({ rgb, weight: w });
  }
  if (!buckets.length) return [58, 49, 40];
  let tw = 0;
  const acc: RGB = [0, 0, 0];
  for (const b of buckets) {
    tw += b.weight;
    acc[0] += b.rgb[0] * b.weight;
    acc[1] += b.rgb[1] * b.weight;
    acc[2] += b.rgb[2] * b.weight;
  }
  return [acc[0] / tw, acc[1] / tw, acc[2] / tw];
}

/** Sample a loaded <img>. Returns null if the canvas is unavailable or tainted. */
export function dominantFromImage(img: HTMLImageElement): RGB | null {
  try {
    const c = document.createElement("canvas");
    c.width = 16;
    c.height = 16;
    const ctx = c.getContext("2d", { willReadFrequently: true });
    if (!ctx) return null;
    ctx.drawImage(img, 0, 0, 16, 16);
    return dominantFromPixels(ctx.getImageData(0, 0, 16, 16).data);
  } catch {
    return null;
  }
}

const imageTintCache = new Map<string, string>();

/** Resolve a CSS color for a header tint from server tint or artwork URL. */
export function tintFromServer(tint: string | null | undefined): string | null {
  if (!tint) return null;
  const rgb = hexToRgb(tint);
  return rgb ? rgbToCss(tameTint(rgb)) : null;
}

export function tintFromImageUrl(url: string): Promise<string | null> {
  const hit = imageTintCache.get(url);
  if (hit) return Promise.resolve(hit);
  return new Promise((resolve) => {
    const img = new Image();
    img.crossOrigin = "anonymous";
    img.decoding = "async";
    img.onload = () => {
      const rgb = dominantFromImage(img);
      const css = rgb ? rgbToCss(tameTint(rgb)) : null;
      if (css) imageTintCache.set(url, css);
      resolve(css);
    };
    img.onerror = () => resolve(null);
    img.src = url;
  });
}
