/**
 * Reduced-motion wiring. Mirrors the OS `prefers-reduced-motion` setting onto
 * `<html data-reduced-motion>` so CSS (tokens.css) zeroes every motion token
 * and kills transitions/animations, and so TS code can ask `prefersReducedMotion()`.
 * The Reduce motion setting can force it on or off instead of following the OS.
 */

const QUERY = "(prefers-reduced-motion: reduce)";

export type MotionPreference = "system" | "on" | "off";

let preference: MotionPreference = "system";
let systemReduced = false;

export function prefersReducedMotion(win: Window = window): boolean {
  return win.document.documentElement.dataset.reducedMotion === "true";
}

export function setReducedMotion(on: boolean, doc: Document = document): void {
  doc.documentElement.dataset.reducedMotion = on ? "true" : "false";
}

function apply(doc: Document) {
  setReducedMotion(preference === "on" || (preference === "system" && systemReduced), doc);
}

/** The Reduce motion setting: follow the OS, or force on or off. */
export function setMotionPreference(p: MotionPreference, doc: Document = document): void {
  preference = p;
  apply(doc);
}

/** Start mirroring the media query. Returns an unsubscribe function. */
export function installReducedMotion(win: Window = window): () => void {
  if (typeof win.matchMedia !== "function") {
    systemReduced = false;
    apply(win.document);
    return () => {};
  }
  const mq = win.matchMedia(QUERY);
  systemReduced = mq.matches;
  apply(win.document);
  const onChange = (e: MediaQueryListEvent) => {
    systemReduced = e.matches;
    apply(win.document);
  };
  mq.addEventListener?.("change", onChange);
  return () => mq.removeEventListener?.("change", onChange);
}

/** Motion duration in ms, or 0 under reduced motion (for JS-driven animation). */
export function duration(ms: number, win: Window = window): number {
  return prefersReducedMotion(win) ? 0 : ms;
}
