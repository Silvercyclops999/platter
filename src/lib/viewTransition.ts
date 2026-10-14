/**
 * Shared-element transition from a grid card's artwork into the detail page
 * header, using the browser's View Transitions API. Falls back to the plain
 * page fade where the API is missing, and skips motion entirely under reduced
 * motion.
 */
import { flushSync } from "react-dom";
import { prefersReducedMotion } from "../design/motion";

/** view-transition-name shared by the clicked card art and the header art. */
export const ART_TRANSITION = "platter-art";

type StartViewTransition = (update: () => Promise<void> | void) => { finished: Promise<void> };

function startFn(doc: Document): StartViewTransition | null {
  const fn = (doc as Document & { startViewTransition?: StartViewTransition }).startViewTransition;
  return typeof fn === "function" ? fn.bind(doc) : null;
}

function reducedMotion(win: Window): boolean {
  if (prefersReducedMotion(win)) return true;
  return typeof win.matchMedia === "function" && win.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

/**
 * Resolve once the new header art has loaded, or after `maxMs`. Polls with
 * timers: rendering (and so requestAnimationFrame) is paused while the
 * transition's update callback runs.
 */
function headerArtReady(doc: Document, maxMs: number): Promise<void> {
  return new Promise((resolve) => {
    const started = performance.now();
    const check = () => {
      if (doc.querySelector(".detail-header__art .art.is-loaded") || performance.now() - started >= maxMs) resolve();
      else setTimeout(check, 16);
    };
    check();
  });
}

/**
 * Run `go` (a navigation) as a view transition that morphs `art` into the
 * next page's header art. Returns true when a transition was started.
 */
export function navigateWithArt(art: HTMLElement | null, go: () => void, win: Window = window): boolean {
  const doc = win.document;
  const start = startFn(doc);
  if (!art || !start || reducedMotion(win)) {
    go();
    return false;
  }
  // Only one element may carry the name in each snapshot: the clicked art in
  // the old one, the header art (set by the page) in the new one.
  doc.querySelectorAll<HTMLElement>(".detail-header__art").forEach((el) => (el.style.viewTransitionName = "none"));
  art.style.viewTransitionName = ART_TRANSITION;
  doc.documentElement.dataset.artTransition = "true";
  const t = start(async () => {
    art.style.viewTransitionName = "";
    flushSync(go);
    await headerArtReady(doc, 250);
  });
  t.finished.finally(() => {
    delete doc.documentElement.dataset.artTransition;
  });
  return true;
}
