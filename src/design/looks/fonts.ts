/**
 * Fonts a look can use. Only fonts shipped with the app, each under the SIL
 * Open Font License; nothing is ever loaded from the internet. Inter is
 * always there; the others load the first time a look uses them (one small
 * Latin file each).
 */
import type { FontId } from "../themes/validate";

export interface FontInfo {
  id: FontId;
  label: string;
  /** The CSS family stack. */
  family: string;
  /** Shown under About. */
  notice: string;
}

const SANS = `system-ui, -apple-system, "Segoe UI", sans-serif`;

export const FONTS: FontInfo[] = [
  { id: "inter", label: "Inter", family: `"Inter Variable", "Inter", ${SANS}`, notice: "Inter typeface" },
  { id: "atkinson", label: "Atkinson Hyperlegible", family: `"Atkinson Hyperlegible", ${SANS}`, notice: "Atkinson Hyperlegible typeface" },
  { id: "manrope", label: "Manrope", family: `"Manrope Variable", ${SANS}`, notice: "Manrope typeface" },
  { id: "fraunces", label: "Fraunces", family: `"Fraunces Variable", Georgia, serif`, notice: "Fraunces typeface" },
];

export function fontInfo(id: FontId): FontInfo {
  return FONTS.find((f) => f.id === id) ?? FONTS[0];
}

type Face = { family: string; url: () => Promise<{ default: string }>; weight: string };

// Each file is bundled with the app; the import only resolves its local URL.
const FACES: Partial<Record<FontId, Face[]>> = {
  atkinson: [
    { family: "Atkinson Hyperlegible", weight: "400", url: () => import("@fontsource/atkinson-hyperlegible/files/atkinson-hyperlegible-latin-400-normal.woff2?url") },
    { family: "Atkinson Hyperlegible", weight: "700", url: () => import("@fontsource/atkinson-hyperlegible/files/atkinson-hyperlegible-latin-700-normal.woff2?url") },
  ],
  manrope: [{ family: "Manrope Variable", weight: "200 800", url: () => import("@fontsource-variable/manrope/files/manrope-latin-wght-normal.woff2?url") }],
  fraunces: [{ family: "Fraunces Variable", weight: "100 900", url: () => import("@fontsource-variable/fraunces/files/fraunces-latin-wght-normal.woff2?url") }],
};

const loaded = new Map<FontId, Promise<void>>();

/** Make a bundled font available. Inter is always loaded. */
export function loadFont(id: FontId, doc: Document = document): Promise<void> {
  const faces = FACES[id];
  if (!faces || typeof FontFace === "undefined" || !doc.fonts) return Promise.resolve();
  let p = loaded.get(id);
  if (!p) {
    p = Promise.all(
      faces.map(async (f) => {
        const { default: url } = await f.url();
        const face = new FontFace(f.family, `url(${JSON.stringify(url)}) format("woff2")`, { weight: f.weight, style: "normal", display: "swap" });
        doc.fonts.add(await face.load());
      }),
    ).then(
      () => undefined,
      () => {
        // A font that can't load falls back to the next in its stack.
        loaded.delete(id);
      },
    );
    loaded.set(id, p);
  }
  return p;
}
