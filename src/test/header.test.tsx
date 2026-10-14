import { act, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Art } from "../components/Art";
import {
  HEADER_TEXT,
  contrastRatio,
  headerColors,
  hexToRgb,
  hslToRgb,
  rgbToHsl,
  vividFromPixels,
  type RGB,
} from "../lib/color";
import { COMPACT_FIRST_STEP, TITLE_STEPS, chooseTitleStep } from "../pages/detail";

// ---------------------------------------------------------------- title steps

describe("header title step", () => {
  // A title 10 px wide per px of font size: 960 px at step 1 (96), 640 at step 2...
  const perPx = 10;

  it("takes the largest step that fits on one line", () => {
    expect(chooseTitleStep(perPx, 2000)).toBe(1);
    expect(chooseTitleStep(perPx, 960)).toBe(1);
    expect(chooseTitleStep(perPx, 959)).toBe(2);
    expect(chooseTitleStep(perPx, 500)).toBe(3);
    expect(chooseTitleStep(perPx, 320)).toBe(4);
  });

  it("falls to the smallest step (which wraps) when nothing fits", () => {
    expect(chooseTitleStep(perPx, 239)).toBe(5);
    expect(chooseTitleStep(perPx, 10)).toBe(5);
    expect(chooseTitleStep(perPx, 0)).toBe(5);
  });

  it("a wider column gives the same title a bigger step", () => {
    const steps = [600, 800, 1000, 1400].map((w) => chooseTitleStep(perPx, w));
    for (let i = 1; i < steps.length; i++) expect(steps[i]).toBeLessThanOrEqual(steps[i - 1]);
  });

  it("the compact header caps the title at step 4", () => {
    expect(COMPACT_FIRST_STEP).toBe(4);
    expect(chooseTitleStep(1, 5000, true)).toBe(4);
    expect(chooseTitleStep(perPx, 300, true)).toBe(5);
  });

  it("has five steps from about 96 down to the 20s", () => {
    expect(TITLE_STEPS).toHaveLength(5);
    expect(TITLE_STEPS[0]).toBeGreaterThanOrEqual(90);
    expect(TITLE_STEPS[4]).toBeGreaterThanOrEqual(20);
    expect(TITLE_STEPS[4]).toBeLessThan(30);
  });
});

// ---------------------------------------------------------------- vivid colour

/** A square RGBA image of `size` px filled with `bg`, with `fill` drawn on top. */
function image(size: number, bg: RGB, fill?: { x: number; y: number; w: number; h: number; rgb: RGB }) {
  const data = new Uint8ClampedArray(size * size * 4);
  for (let y = 0; y < size; y++)
    for (let x = 0; x < size; x++) {
      const inside = fill && x >= fill.x && x < fill.x + fill.w && y >= fill.y && y < fill.y + fill.h;
      const c = inside ? fill!.rgb : bg;
      data.set([c[0], c[1], c[2], 255], (y * size + x) * 4);
    }
  return data;
}

const hueOf = (rgb: RGB) => rgbToHsl(rgb)[0];

describe("header colour picker", () => {
  it("a pale cover with a small red square gives red, not the pale average", () => {
    const px = image(24, [236, 232, 224], { x: 3, y: 3, w: 5, h: 5, rgb: [210, 24, 30] });
    const picked = vividFromPixels(px)!;
    expect(picked).not.toBeNull();
    const h = hueOf(picked);
    expect(h < 10 || h > 350).toBe(true);
    expect(rgbToHsl(picked)[1]).toBeGreaterThan(0.6);
  });

  it("greyscale art gives null, which becomes a neutral grey", () => {
    const px = image(24, [30, 30, 30], { x: 0, y: 0, w: 24, h: 12, rgb: [200, 200, 200] });
    // A bit of JPEG-like noise stays grey.
    for (let i = 0; i < px.length; i += 16) px[i] = Math.min(255, px[i] + 6);
    expect(vividFromPixels(px)).toBeNull();
    const { vivid, shade } = headerColors(null);
    for (const c of [vivid, shade]) {
      const [r, g, b] = c.match(/\d+/g)!.map(Number);
      expect(r).toBe(g);
      expect(g).toBe(b);
    }
  });

  it("an orange cover gives orange", () => {
    const px = image(24, [232, 118, 28], { x: 0, y: 18, w: 24, h: 6, rgb: [20, 16, 12] });
    const h = hueOf(vividFromPixels(px)!);
    expect(h).toBeGreaterThan(18);
    expect(h).toBeLessThan(40);
  });

  it("picks the most vivid cluster over a larger dull one", () => {
    // Mostly dull blue-grey, a smaller saturated green patch.
    const px = image(24, [96, 104, 120], { x: 0, y: 0, w: 8, h: 8, rgb: [30, 200, 60] });
    const h = hueOf(vividFromPixels(px)!);
    expect(h).toBeGreaterThan(110);
    expect(h).toBeLessThan(150);
  });

  it("keeps header text readable (AA) on every hue", () => {
    for (let h = 0; h < 360; h += 15) {
      for (const l of [0.3, 0.5, 0.7]) {
        const { vivid, shade } = headerColors(hslToRgb(h, 1, l));
        const v = vivid.match(/\d+/g)!.map(Number) as RGB;
        const s = shade.match(/\d+/g)!.map(Number) as RGB;
        // Big title over the vivid top: large-text AA.
        expect(contrastRatio(HEADER_TEXT, v), `vivid h${h} l${l}`).toBeGreaterThanOrEqual(3);
        // Shade band is well under half brightness, and small text over it is AA.
        expect(Math.max(...s) / 255, `shade h${h}`).toBeLessThan(0.5);
        expect(contrastRatio(HEADER_TEXT, s)).toBeGreaterThanOrEqual(7);
        // Halfway down the fade (where the type label sits) small text is still AA.
        const mid = v.map((c, i) => (c + s[i]) / 2) as RGB;
        expect(contrastRatio(HEADER_TEXT, mid), `mid h${h} l${l}`).toBeGreaterThanOrEqual(4.5);
      }
    }
  });

  it("keeps the picked hue (vivid, not tamed toward grey)", () => {
    const red = headerColors(hexToRgb("#e01818"));
    const v = red.vivid.match(/\d+/g)!.map(Number) as RGB;
    const [h, s] = rgbToHsl(v);
    expect(h < 8 || h > 352).toBe(true);
    expect(s).toBeGreaterThan(0.6);
  });
});

// ---------------------------------------------------------------- image fade

describe("art fade-in", () => {
  afterEach(() => vi.restoreAllMocks());

  it("fades in an image that loads after mount", () => {
    const { container } = render(<Art path="/library/metadata/1/thumb/1" size={48} />);
    const box = container.querySelector(".art")!;
    const img = container.querySelector("img")!;
    expect(box).not.toHaveClass("is-loaded");
    act(() => {
      img.dispatchEvent(new Event("load"));
    });
    expect(box).toHaveClass("is-loaded");
    expect(box).toHaveClass("is-fading");
  });

  it("shows an already-decoded (cached) image at once, without the fade", () => {
    vi.spyOn(HTMLImageElement.prototype, "complete", "get").mockReturnValue(true);
    vi.spyOn(HTMLImageElement.prototype, "naturalWidth", "get").mockReturnValue(64);
    const { container } = render(<Art path="/library/metadata/2/thumb/1" size={48} />);
    const box = container.querySelector(".art")!;
    expect(box).toHaveClass("is-loaded");
    expect(box).not.toHaveClass("is-fading");
    expect(container.querySelector(".art__skeleton")).toBeNull();
    // The load event that follows doesn't start a fade either.
    act(() => {
      container.querySelector("img")!.dispatchEvent(new Event("load"));
    });
    expect(box).not.toHaveClass("is-fading");
  });
});
