/**
 * Reduced-motion wiring. Mirrors the OS `prefers-reduced-motion` setting onto
 * `<html data-reduced-motion>` so CSS (tokens.css) zeroes every motion token
 * and kills transitions/animations, and so TS code can ask `prefersReducedMotion()`.
 */

const QUERY = "(prefers-reduced-motion: reduce)";

export function prefersReducedMotion(win: Window = window): boolean {
  return win.document.documentElement.dataset.reducedMotion === "true";
}

export function setReducedMotion(on: boolean, doc: Document = document): void {
  doc.documentElement.dataset.reducedMotion = on ? "true" : "false";
}

/** Start mirroring the media query. Returns an unsubscribe function. */
export function installReducedMotion(win: Window = window): () => void {
  if (typeof win.matchMedia !== "function") {
    setReducedMotion(false, win.document);
    return () => {};
  }
  const mq = win.matchMedia(QUERY);
  setReducedMotion(mq.matches, win.document);
  const onChange = (e: MediaQueryListEvent) => setReducedMotion(e.matches, win.document);
  mq.addEventListener?.("change", onChange);
  return () => mq.removeEventListener?.("change", onChange);
}

/** Motion duration in ms, or 0 under reduced motion (for JS-driven animation). */
export function duration(ms: number, win: Window = window): number {
  return prefersReducedMotion(win) ? 0 : ms;
}
