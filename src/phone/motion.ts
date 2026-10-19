/**
 * How the phone moves: the timings and curve every transition shares, the
 * edge swipe's thresholds, a small runner over the Web Animations API, and
 * the shared-cover flight between the mini player and now playing.
 *
 * Everything here moves transform and opacity only, sets will-change only
 * while something moves, and runs nothing while the screen is still. Under
 * reduced motion movement becomes a quick fade.
 */
import { prefersReducedMotion } from "../design/motion";

/** The phone's one curve: quick out, a long soft landing, like a spring. Matches --ph-ease. */
export const EASE = "cubic-bezier(0.32, 0.72, 0, 1)";
/** A page sliding in over another, or back off it. */
export const PUSH_MS = 350;
/** One tab giving way to another. */
export const TAB_FADE_MS = 150;
/** Now playing rising from the mini player, cover and all. */
export const NP_OPEN_MS = 380;
/** What every movement becomes under reduced motion. */
export const FADE_MS = 120;
/** The page underneath shifts left by this share of the width. */
export const UNDER_SHIFT = 0.3;

/** A press that starts this close to the left edge can swipe back. */
export const EDGE_PX = 20;
/** Letting go past this share of the width goes back. */
export const BACK_FRACTION = 1 / 3;
/** A flick to the right this fast (px per ms) goes back however short it was. */
export const BACK_FLICK_SPEED = 0.5;
/** The finger moves this far before the swipe decides it is sideways. */
export const EDGE_LOCK_PX = 8;

export type EdgeOutcome = "back" | "stay";

/** What letting go of an edge swipe does. `dx` is how far right the page moved, `vx` its speed. */
export function edgeRelease(g: { dx: number; vx: number; width: number }): EdgeOutcome {
  if (g.dx <= 0) return "stay";
  if (g.dx >= g.width * BACK_FRACTION) return "back";
  if (g.vx >= BACK_FLICK_SPEED && g.dx >= 16) return "back";
  return "stay";
}

/**
 * Whether a press near the edge is a back swipe: once it has moved, it has
 * to be heading right and more sideways than up or down. `null` while it
 * hasn't moved far enough to tell.
 */
export function edgeClaims(dx: number, dy: number): boolean | null {
  if (Math.hypot(dx, dy) < EDGE_LOCK_PX) return null;
  return dx > 0 && Math.abs(dx) > Math.abs(dy) * 1.2;
}

/**
 * Notes the page stack reads on its next change: a tab was picked (so the
 * pages cross-fade), or an edge swipe already slid the page away (so the
 * back needs no animation of its own). `toTop` scrolls the page showing to
 * its top, for a tap on the tab you're on.
 */
export const stackSignals: { tabSwitch: { from: string; to: string } | null; swiped: boolean; toTop: (() => void) | null } = {
  tabSwitch: null,
  swiped: false,
  toTop: null,
};

export const reduced = (): boolean => (typeof document === "undefined" ? false : prefersReducedMotion());

/**
 * Runs one animation and resolves when it ends or is cancelled. Where the
 * Web Animations API is missing it resolves at once (the end state).
 */
export function run(el: Element | null | undefined, frames: Keyframe[], ms: number, opts: { fill?: FillMode; easing?: string } = {}): { done: Promise<void>; cancel: () => void } {
  if (!el || typeof (el as HTMLElement).animate !== "function" || ms <= 0) return { done: Promise.resolve(), cancel: () => {} };
  const node = el as HTMLElement;
  node.style.willChange = "transform, opacity";
  const a = node.animate(frames, { duration: ms, easing: opts.easing ?? EASE, fill: opts.fill ?? "forwards" });
  const clear = () => {
    node.style.willChange = "";
  };
  const done = (a.finished ?? Promise.resolve()).then(clear, clear);
  return {
    done,
    cancel: () => {
      clear();
      a.cancel();
    },
  };
}

/** Drops the hold a finished animation leaves on an element (so fixed children work again). */
export function release(el: Element | null | undefined) {
  const node = el as HTMLElement | null | undefined;
  node?.getAnimations?.().forEach((a) => a.cancel());
  if (node) node.style.willChange = "";
}

/** The rectangle of `el` against `root`'s top left. */
function local(r: { left: number; top: number; width: number; height: number }, root: DOMRect) {
  return { x: r.left - root.left, y: r.top - root.top, w: r.width, h: r.height };
}

/**
 * The cover's flight: a floating copy of `art` sits at the big cover's place
 * and is scaled down to the small art's place, then grows into place
 * (opening) or shrinks back to it (closing). The real cover is hidden while
 * the copy flies. Returns when it lands.
 */
export async function flyCover(opts: {
  root: HTMLElement;
  small: Element;
  big: HTMLElement;
  /** The big cover's resting rectangle, when it isn't where it sits now. */
  bigRect?: { left: number; top: number; width: number; height: number };
  direction: "open" | "close";
  ms: number;
}): Promise<void> {
  const { root, small, big, direction, ms } = opts;
  if (typeof big.animate !== "function" || reduced()) return;
  const rootRect = root.getBoundingClientRect();
  const s = local(small.getBoundingClientRect(), rootRect);
  const b = local(opts.bigRect ?? big.getBoundingClientRect(), rootRect);
  if (!s.w || !b.w) return;
  const copy = document.createElement("div");
  copy.className = "ph-flight";
  copy.setAttribute("aria-hidden", "true");
  copy.style.cssText = `left:${b.x}px;top:${b.y}px;width:${b.w}px;height:${b.h}px;`;
  // The small art first: it is loaded already; the big one covers it once it is.
  const under = small.cloneNode(true) as HTMLElement;
  under.className += " ph-flight__under";
  copy.appendChild(under);
  const bigArt = big.querySelector(".art");
  const bigImg = bigArt?.querySelector("img");
  if (bigArt && bigImg?.complete && bigImg.naturalWidth > 0) copy.appendChild(bigArt.cloneNode(true));
  root.appendChild(copy);
  const shrunk = `translate(${s.x - b.x}px, ${s.y - b.y}px) scale(${s.w / b.w}, ${s.h / b.h})`;
  const frames: Keyframe[] = direction === "open" ? [{ transform: shrunk }, { transform: "none" }] : [{ transform: "none" }, { transform: shrunk }];
  big.style.opacity = "0";
  const a = run(copy, frames, ms);
  await a.done;
  big.style.opacity = "";
  copy.remove();
}
