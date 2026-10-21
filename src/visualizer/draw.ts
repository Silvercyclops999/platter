/**
 * What every visualizer style draws with: an animation loop capped at a
 * frame rate (and stopped while the page is hidden), and the reduced motion
 * flag as React state.
 */
import { useEffect, useLayoutEffect, useRef, useSyncExternalStore } from "react";
import { prefersReducedMotion } from "../design/motion";

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
