/**
 * The artwork's two gestures on the phone's now playing screen: a sideways
 * swipe skips (left for the next song, right for the previous one) and a
 * pull down closes the player. The art follows the finger; letting go short
 * of the distance, without a flick, snaps it back. Nothing else (no swipe up).
 * The hook only reports; NowPlaying's cover deck moves things.
 */
import { useRef, useState, type PointerEvent as ReactPointerEvent } from "react";

/** A swipe past this share of the width skips. */
export const SKIP_FRACTION = 0.3;
/** A pull past this share of the height closes. */
export const CLOSE_FRACTION = 0.25;
/** A flick at this speed (px per ms) counts however short it was. */
export const FLICK_SPEED = 0.6;
/** A flick still has to travel this far, so a tap never skips. */
export const FLICK_MIN_PX = 24;
/** The finger has to move this far before the gesture picks a direction. */
export const AXIS_LOCK_PX = 10;
/** Pulling up gives this much (rubber band), then stops. */
export const UP_GIVE_PX = 24;

export type Axis = "x" | "y";
export type Outcome = "next" | "previous" | "close" | "snap";

/** Which way the gesture goes, once the finger has moved far enough. */
export function lockAxis(dx: number, dy: number): Axis | null {
  if (Math.hypot(dx, dy) < AXIS_LOCK_PX) return null;
  return Math.abs(dx) > Math.abs(dy) ? "x" : "y";
}

/** What letting go does. Velocities are px per ms, positive right and down. */
export function release(g: { axis: Axis | null; dx: number; dy: number; vx: number; vy: number; width: number; height: number }): Outcome {
  if (g.axis === "x") {
    const far = Math.abs(g.dx) >= g.width * SKIP_FRACTION;
    const flick = Math.abs(g.vx) >= FLICK_SPEED && Math.abs(g.dx) >= FLICK_MIN_PX && Math.sign(g.vx) === Math.sign(g.dx);
    if (far || flick) return g.dx < 0 ? "next" : "previous";
    return "snap";
  }
  if (g.axis === "y" && g.dy > 0) {
    const far = g.dy >= g.height * CLOSE_FRACTION;
    const flick = g.vy >= FLICK_SPEED && g.dy >= FLICK_MIN_PX;
    if (far || flick) return "close";
  }
  return "snap";
}

/** Where the art sits while the finger is down: along the locked axis only. */
export function follow(axis: Axis | null, dx: number, dy: number): { x: number; y: number } {
  if (axis === "x") return { x: dx, y: 0 };
  if (axis === "y") return { x: 0, y: dy >= 0 ? dy : Math.max(-UP_GIVE_PX, dy / 3) };
  return { x: 0, y: 0 };
}

interface Track {
  id: number;
  x0: number;
  y0: number;
  axis: Axis | null;
  last: { x: number; y: number; t: number };
  v: { x: number; y: number };
}

/** Where a drag of the art ended up when the finger lifted, and how fast it was going. */
export interface ArtRelease {
  axis: Axis | null;
  dx: number;
  dy: number;
  vx: number;
  vy: number;
  /** Where the art or the screen sat when the finger lifted (follow()). */
  at: { x: number; y: number };
}

/**
 * Pointer handlers for the artwork. Nothing here renders: every move goes
 * straight to `place` (which writes a transform), and letting go hands the
 * outcome to `done`, so a drag never redraws the screen. `size` gives the
 * width (for skips) and the screen height (for the pull). The handlers keep
 * their identity across renders.
 */
export function useArtGestures(opts: {
  size: () => { width: number; height: number };
  /** The finger moved: put the art (x) or the screen (y) here. */
  place: (at: { x: number; y: number }, axis: Axis) => void;
  /** The finger lifted or was cancelled (a cancel is always "snap"). */
  done: (outcome: Outcome, g: ArtRelease) => void;
  /** A press began (so a settle still running can finish first). */
  start?: () => void;
}) {
  const t = useRef<Track | null>(null);
  const o = useRef(opts);
  o.current = opts;
  const [handlers] = useState(() => {
    const now = () => (typeof performance !== "undefined" ? performance.now() : Date.now());
    const onPointerDown = (e: ReactPointerEvent<HTMLElement>) => {
      if (e.button !== 0 && e.pointerType === "mouse") return;
      o.current.start?.();
      t.current = { id: e.pointerId, x0: e.clientX, y0: e.clientY, axis: null, last: { x: e.clientX, y: e.clientY, t: now() }, v: { x: 0, y: 0 } };
      e.currentTarget.setPointerCapture?.(e.pointerId);
    };
    const onPointerMove = (e: ReactPointerEvent<HTMLElement>) => {
      const g = t.current;
      if (!g || g.id !== e.pointerId) return;
      const dx = e.clientX - g.x0;
      const dy = e.clientY - g.y0;
      g.axis ??= lockAxis(dx, dy);
      const at = now();
      const dt = Math.max(1, at - g.last.t);
      g.v = { x: (e.clientX - g.last.x) / dt, y: (e.clientY - g.last.y) / dt };
      g.last = { x: e.clientX, y: e.clientY, t: at };
      if (g.axis) o.current.place(follow(g.axis, dx, dy), g.axis);
    };
    const end = (e: ReactPointerEvent<HTMLElement>, cancelled: boolean) => {
      const g = t.current;
      if (!g || g.id !== e.pointerId) return;
      t.current = null;
      const { width, height } = o.current.size();
      const dx = e.clientX - g.x0;
      const dy = e.clientY - g.y0;
      const outcome = cancelled ? "snap" : release({ axis: g.axis, dx, dy, vx: g.v.x, vy: g.v.y, width, height });
      o.current.done(outcome, { axis: g.axis, dx, dy, vx: g.v.x, vy: g.v.y, at: follow(g.axis, dx, dy) });
    };
    return {
      onPointerDown,
      onPointerMove,
      onPointerUp: (e: ReactPointerEvent<HTMLElement>) => end(e, false),
      onPointerCancel: (e: ReactPointerEvent<HTMLElement>) => end(e, true),
    };
  });
  return handlers;
}

/**
 * A press that moves farther than this is a swipe or a scroll, never a tap.
 * A still press can last as long as it likes. (8 was tight enough that an
 * ordinary thumb tap sometimes failed it and needed a second try.)
 */
export const TAP_SLOP_PX = 10;

/** A tap shorter than this still shows its press for this long. */
export const PRESS_MIN_MS = 90;

/** The class a control wears while a finger is on it. */
export const PRESSED = "is-pressed";

/** What answers a press: buttons, links and anything acting as one. Sliders and drag handles don't. */
const PRESSABLE = 'button, a[href], [role="button"], [role="tab"], [role="switch"], [role="radio"], [role="checkbox"], [role="option"], [role="menuitem"]';

/** The control a press at `target` lights, if any. */
export function pressTarget(target: EventTarget | null, root: HTMLElement): HTMLElement | null {
  if (!(target instanceof Element)) return null;
  const el = target.closest<HTMLElement>(PRESSABLE);
  if (!el || !root.contains(el)) return null;
  if (el.matches(':disabled, [aria-disabled="true"], .ph-handle, [data-no-press]')) return null;
  return el;
}

/**
 * Keeps swipes from acting as taps anywhere under `root`: a press that
 * moved more than TAP_SLOP_PX, or during which anything scrolled, doesn't
 * click what it started on. The control under the finger wears PRESSED
 * from the moment it goes down until it lifts, and loses it as soon as the
 * press turns into a swipe or a scroll. Returns a function that removes it.
 */
export function installTapGuard(root: HTMLElement): () => void {
  let start: { x: number; y: number; id: number; target: Node | null; scrolls: Map<Element, [number, number]> } | null = null;
  let moved = false;
  let lit: HTMLElement | null = null;
  let litAt = 0;
  const now = () => (typeof performance !== "undefined" ? performance.now() : Date.now());
  const unlight = () => {
    lit?.classList.remove(PRESSED);
    lit = null;
  };
  const down = (e: PointerEvent) => {
    // Where every box around the press was scrolled, so only a real scroll (not the
    // last tick of one that is settling) turns the press into a scroll.
    const scrolls = new Map<Element, [number, number]>();
    for (let el = e.target instanceof Element ? e.target : null; el; el = el.parentElement) scrolls.set(el, [el.scrollTop, el.scrollLeft]);
    const doc = document.scrollingElement;
    if (doc) scrolls.set(doc, [doc.scrollTop, doc.scrollLeft]);
    start = { x: e.clientX, y: e.clientY, id: e.pointerId, target: e.target instanceof Node ? e.target : null, scrolls };
    moved = false;
    unlight();
    if (e.pointerType === "mouse" && e.button !== 0) return;
    lit = pressTarget(e.target, root);
    litAt = now();
    lit?.classList.add(PRESSED);
  };
  const move = (e: PointerEvent) => {
    if (!start || e.pointerId !== start.id || moved) return;
    if (Math.hypot(e.clientX - start.x, e.clientY - start.y) > TAP_SLOP_PX) {
      moved = true;
      unlight();
    }
  };
  // Only a scroll of something the press is inside counts: a page still
  // settling elsewhere (or the one under a sheet) mustn't eat a tab's tap.
  const scrolled = (e: Event) => {
    const box = e.target === document ? (document.scrollingElement ?? document.documentElement) : e.target;
    if (!start || !(box instanceof Element) || !start.target || !box.contains(start.target)) return;
    const was = start.scrolls.get(box);
    const far = was ? Math.abs(box.scrollTop - was[0]) + Math.abs(box.scrollLeft - was[1]) : Infinity;
    if (far > TAP_SLOP_PX) {
      moved = true;
      unlight();
    }

  };
  const click = (e: MouseEvent) => {
    if (!moved) return;
    moved = false;
    e.preventDefault();
    e.stopPropagation();
  };
  // A quick tap still shows: the light stays at least PRESS_MIN_MS.
  const up = (e: PointerEvent) => {
    if (start && e.pointerId !== start.id) return;
    start = null;
    const el = lit;
    if (!el) return;
    lit = null;
    const held = now() - litAt;
    if (e.type === "pointercancel" || held >= PRESS_MIN_MS) el.classList.remove(PRESSED);
    else setTimeout(() => el !== lit && el.classList.remove(PRESSED), PRESS_MIN_MS - held);
  };
  root.addEventListener("pointerdown", down, true);
  root.addEventListener("pointermove", move, true);
  root.addEventListener("pointerup", up, true);
  root.addEventListener("pointercancel", up, true);
  root.addEventListener("scroll", scrolled, true);
  root.addEventListener("click", click, true);
  return () => {
    unlight();
    root.removeEventListener("pointerdown", down, true);
    root.removeEventListener("pointermove", move, true);
    root.removeEventListener("pointerup", up, true);
    root.removeEventListener("pointercancel", up, true);
    root.removeEventListener("scroll", scrolled, true);
    root.removeEventListener("click", click, true);
  };
}
