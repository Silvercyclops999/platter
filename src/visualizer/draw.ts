/**
 * What every visualizer style draws with: a canvas sized to its box at the
 * screen's pixel density, an animation loop capped at a frame rate (and
 * stopped while the page is hidden), and the reduced motion flag as React
 * state.
 */
import { useEffect, useLayoutEffect, useRef, useState, useSyncExternalStore, type RefObject } from "react";
import { prefersReducedMotion } from "../design/motion";

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

/**
 * Sizes a canvas's backing store to `w` x `h` CSS pixels at `scale` times
 * the screen's density (capped at 2x: nothing here needs a 3x screen's
 * pixels). Returns the backing pixels per CSS pixel.
 */
export function fitCanvas(canvas: HTMLCanvasElement, w: number, h: number, scale = 1): number {
  const dpr = Math.min(2, typeof window !== "undefined" ? window.devicePixelRatio || 1 : 1) * scale;
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
  // The latest draw, kept after each render (never written during one).
  useLayoutEffect(() => {
    fn.current = draw;
  });
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
