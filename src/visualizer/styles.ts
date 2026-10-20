/**
 * The visualizer's styles in their two groups, which ones reduced motion
 * keeps, and the frame rate each place runs at. Plain data and small
 * functions, so the picker, the Settings row and the views agree.
 */
import type { VisualizerIntensity, VisualizerStyle } from "../lib/types";

export type ArtStyle = "sleeve" | "ink" | "grooves" | "light";
export type InstrumentStyle = "analyser" | "meters" | "scope";

export interface StyleChoice<T extends VisualizerStyle = VisualizerStyle> {
  id: T;
  label: string;
}

export const ART_STYLES: readonly StyleChoice<ArtStyle>[] = [
  { id: "sleeve", label: "Sleeve" },
  { id: "ink", label: "Ink" },
  { id: "grooves", label: "Grooves" },
  { id: "light", label: "Light" },
];

export const INSTRUMENT_STYLES: readonly StyleChoice<InstrumentStyle>[] = [
  { id: "analyser", label: "Analyser" },
  { id: "meters", label: "Meters" },
  { id: "scope", label: "Scope" },
];

export const STYLE_GROUPS: readonly { name: "Art" | "Instruments"; styles: readonly StyleChoice[] }[] = [
  { name: "Art", styles: ART_STYLES },
  { name: "Instruments", styles: INSTRUMENT_STYLES },
];

/** Every style, Art first. */
export const STYLES: readonly StyleChoice[] = [...ART_STYLES, ...INSTRUMENT_STYLES];

export const DEFAULT_STYLE: VisualizerStyle = "sleeve";

const KNOWN = new Set<string>(STYLES.map((s) => s.id));

/**
 * A saved style as it is today: Glow became part of Sleeve, and anything
 * unknown (or nothing) is the default.
 */
export function normalizeStyle(v: string | null | undefined): VisualizerStyle {
  if (v === "glow") return "sleeve";
  return v && KNOWN.has(v) ? (v as VisualizerStyle) : DEFAULT_STYLE;
}

export const isArtStyle = (s: VisualizerStyle): s is ArtStyle => s === "sleeve" || s === "ink" || s === "grooves" || s === "light";

/** The styles reduced motion keeps: a still Sleeve, and the two instruments that read as gauges. */
export const CALM_STYLES: readonly VisualizerStyle[] = ["sleeve", "analyser", "meters"];
/** Frames a second under reduced motion. */
export const CALM_FPS = 15;
/** The phone's most. */
export const PHONE_MAX_FPS = 30;

/** The style to draw: the chosen one, unless reduced motion leaves it out (then a still Sleeve). */
export function shownStyle(chosen: string | null | undefined, reduced: boolean): VisualizerStyle {
  const s = normalizeStyle(chosen);
  return reduced && !CALM_STYLES.includes(s) ? "sleeve" : s;
}

/** Frames a second: the setting, capped on a phone, 15 under reduced motion. */
export function visFps(setting: number | undefined, phone: boolean, reduced: boolean): number {
  if (reduced) return CALM_FPS;
  const want = setting ?? (phone ? PHONE_MAX_FPS : 60);
  return phone ? Math.min(PHONE_MAX_FPS, want) : want;
}

export const INTENSITY_CHOICES: readonly { value: VisualizerIntensity; label: string }[] = [
  { value: "calm", label: "Calm" },
  { value: "lively", label: "Lively" },
];
