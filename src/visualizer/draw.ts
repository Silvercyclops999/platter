/**
 * What every visualizer style draws with: a canvas sized to its box at the
 * screen's pixel density, an animation loop capped at the frame rate setting
 * (and stopped while the page is hidden), the cover's accent, and the
 * reduced motion flag as React state.
 */
import { useEffect, useRef, useState, useSyncExternalStore, type RefObject } from "react";
import { coverLightColour } from "../design/coverLight";
import { prefersReducedMotion } from "../design/motion";
import { useCoverColors } from "../shell/nowPlaying";
import { usePlayer } from "../stores/player";
import { useSettings } from "../stores/settings";
import { useThemeBase } from "../stores/theme";

/** A box's size in CSS pixels, followed as it changes. */
export function useBoxSize(ref: RefObject<HTMLElement | null>): { w: number; h: number } {
  const [size, setSize] = useState({ w: 0, h: 0 });
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const read = () => {
      const r = el.getBoundingClientRect();
      setSize((s) => (Math.round(r.width) === s.w && Math.round(r.height) === s.h ? s : { w: Math.round(r.width), h: Math.round(r.height) }));
    };
    read();
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(read);
    ro.observe(el);
    return () => ro.disconnect();
  }, [ref]);
  return size;
}

/** Sizes a canvas's backing store to `w` x `h` CSS pixels at the screen's density. Returns the scale. */
export function fitCanvas(canvas: HTMLCanvasElement, w: number, h: number): number {
  const dpr = Math.min(3, typeof window !== "undefined" ? window.devicePixelRatio || 1 : 1);
  const bw = Math.max(1, Math.round(w * dpr));
  const bh = Math.max(1, Math.round(h * dpr));
  if (canvas.width !== bw) canvas.width = bw;
  if (canvas.height !== bh) canvas.height = bh;
  return dpr;
}

/**
 * Calls `draw(dtMs, now)` on animation frames, at most `fps` times a
 * second. Animation frames don't run while the page is hidden, so nothing
 * runs then either.
 */
export function useFrameLoop(draw: (dtMs: number, now: number) => void, fps: number, on = true): void {
  const fn = useRef(draw);
  fn.current = draw;
  useEffect(() => {
    if (!on || typeof requestAnimationFrame === "undefined") return;
    const every = 1000 / Math.max(1, fps);
    let id = 0;
    let lastDraw = -Infinity;
    let prev = performance.now();
    const tick = (now: number) => {
      id = requestAnimationFrame(tick);
      // A little slack, so 60 on a 60 Hz screen draws every frame.
      if (now - lastDraw < every - 2) return;
      const dt = Math.min(250, now - prev);
      prev = now;
      lastDraw = now;
      fn.current(dt, now);
    };
    id = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(id);
  }, [fps, on]);
}

function subscribeMotion(cb: () => void): () => void {
  if (typeof MutationObserver === "undefined") return () => {};
  const mo = new MutationObserver(cb);
  mo.observe(document.documentElement, { attributes: true, attributeFilter: ["data-reduced-motion"] });
  return () => mo.disconnect();
}

/** Reduced motion, as state (the app's setting or the system's). */
export function useReducedMotion(): boolean {
  return useSyncExternalStore(subscribeMotion, () => prefersReducedMotion(), () => false);
}

/**
 * The line colour: the playing cover's colour, held to a band that reads on
 * the look's surface (or on `over`, for views that are always dark), or null
 * for the look's own accent (no cover, or Artwork colours off).
 */
export function useVisAccent(over?: "dark" | "light"): string | null {
  const thumb = usePlayer((s) => s.state?.track?.thumb ?? null);
  const colours = useSettings((s) => s.settings?.artworkColours !== false);
  const look = useThemeBase();
  const c = useCoverColors(colours ? thumb : null);
  return c ? coverLightColour(c.vivid, over ?? look) : null;
}

/** A CSS custom property's value on `el`, or `fallback`. */
export function cssVar(el: Element, name: string, fallback: string): string {
  const v = getComputedStyle(el).getPropertyValue(name).trim();
  return v || fallback;
}

/** The look's colours a canvas needs. */
export interface LookColours {
  accent: string;
  muted: string;
}

/**
 * The look's accent and muted text on `ref`'s element, read once and again
 * when the look changes (never per frame: reading styles costs a layout).
 */
export function useLookColours(ref: RefObject<HTMLElement | null>): RefObject<LookColours> {
  const colours = useRef<LookColours>({ accent: "#ECE4D3", muted: "#8A8178" });
  useEffect(() => {
    const read = () => {
      const el = ref.current;
      if (!el) return;
      colours.current = { accent: cssVar(el, "--color-accent", "#ECE4D3"), muted: cssVar(el, "--color-text-muted", "#8A8178") };
    };
    read();
    if (typeof MutationObserver === "undefined") return;
    const mo = new MutationObserver(read);
    mo.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme", "data-theme-base", "style", "class"] });
    return () => mo.disconnect();
  }, [ref]);
  return colours;
}
