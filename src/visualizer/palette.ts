/**
 * A cover's palette for the art styles: four colours and a deep base,
 * picked from the image itself. The lead is the same most vivid colour the
 * page headers and now playing's tint use (vividFromPixels), so the art
 * agrees with the rest of the app; the others are the next strongest hues
 * the cover really has, then its light and shade. A grey cover gets warm
 * neutrals from its own tones rather than invented colour.
 */
import { hslToRgb, rgbToHsl, vividFromPixels, type RGB } from "../lib/color";

export interface ArtPalette {
  /** Four colours, the lead first, each 0..1 per channel. */
  colours: [RGB, RGB, RGB, RGB];
  /** A deep tone from the cover's darkest parts, for the art's shadows. */
  base: RGB;
}

const BUCKETS = 18;
const MIN_CHROMA = 0.1;

/** Held to a band that glows on a dark screen without going neon or muddy. */
function tame([r, g, b]: RGB, grey: boolean): RGB {
  const [h, s, l] = rgbToHsl([r, g, b]);
  const sat = grey ? Math.min(0.16, s) : Math.min(0.85, Math.max(0.38, s * 1.1));
  const light = Math.min(0.68, Math.max(0.34, l));
  return hslToRgb(h, sat, light);
}

const unit = (c: RGB): RGB => [c[0] / 255, c[1] / 255, c[2] / 255];

const hueDistance = (a: number, b: number) => {
  const d = Math.abs(a - b) % 360;
  return d > 180 ? 360 - d : d;
};

/** The palette of an RGBA buffer (a cover scaled down to a few hundred pixels). */
export function paletteFromPixels(data: Uint8ClampedArray | number[]): ArtPalette {
  const weight = new Float64Array(BUCKETS);
  const sum = Array.from({ length: BUCKETS }, () => [0, 0, 0]);
  const tones: [number, RGB][] = [];
  for (let i = 0; i < data.length; i += 4) {
    if (data[i + 3] < 128) continue;
    const rgb: RGB = [data[i], data[i + 1], data[i + 2]];
    const [h, s, l] = rgbToHsl(rgb);
    tones.push([l, rgb]);
    const chroma = (Math.max(...rgb) - Math.min(...rgb)) / 255;
    if (chroma < MIN_CHROMA) continue;
    const w = s * chroma;
    const k = Math.floor(h / (360 / BUCKETS)) % BUCKETS;
    weight[k] += w;
    sum[k][0] += rgb[0] * w;
    sum[k][1] += rgb[1] * w;
    sum[k][2] += rgb[2] * w;
  }
  tones.sort((a, b) => a[0] - b[0]);
  const avg = (from: number, to: number): RGB => {
    const part = tones.slice(Math.floor(from * tones.length), Math.max(Math.floor(from * tones.length) + 1, Math.floor(to * tones.length)));
    if (!part.length) return [40, 36, 32];
    const s = part.reduce((acc, [, c]) => [acc[0] + c[0], acc[1] + c[1], acc[2] + c[2]], [0, 0, 0]);
    return [s[0] / part.length, s[1] / part.length, s[2] / part.length];
  };

  const lead = vividFromPixels(data);
  const grey = !lead;
  const picked: RGB[] = [];
  if (lead) {
    picked.push(lead);
    // The next strongest hues, each at least 40 degrees from the ones picked.
    const order = [...weight.keys()].filter((k) => weight[k] > 0).sort((a, b) => weight[b] - weight[a]);
    const total = weight.reduce((a, b) => a + b, 0);
    for (const k of order) {
      if (picked.length >= 3) break;
      if (weight[k] < total * 0.04) break;
      const c: RGB = [sum[k][0] / weight[k], sum[k][1] / weight[k], sum[k][2] / weight[k]];
      const h = rgbToHsl(c)[0];
      if (picked.every((p) => hueDistance(rgbToHsl(p)[0], h) >= 40)) picked.push(c);
    }
  }
  // Then the cover's own light and shade, tinted by the lead.
  const light = avg(0.7, 0.95);
  const mid = avg(0.35, 0.65);
  const fill: RGB[] = [light, mid, avg(0.15, 0.35)];
  if (lead) {
    const [h, s] = rgbToHsl(lead);
    fill.unshift(hslToRgb((h + 28) % 360, s * 0.8, 0.5), hslToRgb((h + 332) % 360, s * 0.75, 0.42));
  }
  for (const c of fill) {
    if (picked.length >= 4) break;
    picked.push(c);
  }
  const colours = picked.slice(0, 4).map((c) => unit(tame(c, grey))) as ArtPalette["colours"];
  // The base: the darkest quarter, darker still, keeping a little of its hue.
  const dark = avg(0, 0.25);
  const [h, s, l] = rgbToHsl(dark);
  const base = unit(hslToRgb(h, Math.min(0.5, s), Math.min(0.09, Math.max(0.03, l * 0.5))));
  return { colours, base };
}

/** A palette from the lead colour alone (the cover couldn't be read): its hue turned a little each way. */
export function paletteFromColour(lead: RGB | null): ArtPalette {
  if (!lead) {
    const warm: RGB[] = [
      [0.62, 0.55, 0.47],
      [0.42, 0.38, 0.34],
      [0.74, 0.68, 0.6],
      [0.3, 0.27, 0.25],
    ];
    return { colours: warm as ArtPalette["colours"], base: [0.04, 0.035, 0.03] };
  }
  const [h, s] = rgbToHsl(lead);
  const at = (dh: number, ds: number, l: number) => unit(tame(hslToRgb((h + dh + 360) % 360, s * ds, l), false));
  return {
    colours: [at(0, 1, 0.5), at(30, 0.85, 0.46), at(-35, 0.8, 0.4), at(12, 0.5, 0.64)],
    base: unit(hslToRgb(h, Math.min(0.5, s), 0.05)),
  };
}

/** Side of the square a cover is scaled to before its palette is read. */
export const PALETTE_SAMPLE = 32;

/** The palette of a loaded image, or null when it can't be read (no canvas, or a cover from another origin). */
export function paletteFromImage(img: CanvasImageSource): ArtPalette | null {
  try {
    const c = document.createElement("canvas");
    c.width = PALETTE_SAMPLE;
    c.height = PALETTE_SAMPLE;
    const ctx = c.getContext("2d", { willReadFrequently: true });
    if (!ctx) return null;
    ctx.drawImage(img, 0, 0, PALETTE_SAMPLE, PALETTE_SAMPLE);
    return paletteFromPixels(ctx.getImageData(0, 0, PALETTE_SAMPLE, PALETTE_SAMPLE).data);
  } catch {
    return null;
  }
}
