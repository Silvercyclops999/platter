/**
 * The visualizer's styles (Cover is just the album art, nothing moving), in the order a swipe or an arrow key goes
 * through them, and the frame rate each place runs at.
 */
import type { VisualizerStyle } from "../lib/types";

export interface StyleChoice {
  id: VisualizerStyle;
  label: string;
}

export const STYLES: readonly StyleChoice[] = [
  { id: "grooves", label: "Grooves" },
  { id: "blur", label: "Blur" },
  { id: "bars", label: "Bars" },
  { id: "cover", label: "Cover" },
];

export const DEFAULT_STYLE: VisualizerStyle = "grooves";

const KNOWN = new Set<string>(STYLES.map((s) => s.id));

/** A saved style as it is today: anything from before the rework (or nothing) is Grooves. */
export function normalizeStyle(v: string | null | undefined): VisualizerStyle {
  return v && KNOWN.has(v) ? (v as VisualizerStyle) : DEFAULT_STYLE;
}

/** The style `step` places on from `from`, wrapping round (+1 is the next one). */
export function stepStyle(from: string | null | undefined, step: 1 | -1): VisualizerStyle {
  const i = STYLES.findIndex((s) => s.id === normalizeStyle(from));
  return STYLES[(i + step + STYLES.length) % STYLES.length].id;
}

export const styleLabel = (id: VisualizerStyle) => STYLES.find((s) => s.id === id)?.label ?? id;

/** Frames a second on a computer, and the phone's most. */
export const DESKTOP_FPS = 60;
export const PHONE_FPS = 30;
/** Paused, the art moves at most this many frames a second. */
export const PAUSED_FPS = 12;
/** How many bars Bars draws across the cover. */
export const PHONE_BARS = 20;
export const DESKTOP_BARS = 32;
