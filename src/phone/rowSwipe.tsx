/**
 * Swipe a row sideways to act on it. A song row swipes right into the queue
 * and left into (or out of) Liked Songs; a queue row swipes left out of the
 * queue. The row follows the finger, the action's mark shows under it, and
 * letting go past SWIPE_MARK_PX runs the action (a light tick as the mark
 * lights, a short buzz on letting go); the row slides back either way.
 *
 * A press that heads up or down first is a scroll and the row stays put
 * (the row only takes sideways moves, see .ph-swipe in phone.css). A press
 * at the very left edge is left to the back swipe. A direction the row has
 * no action for is left alone too (a shelf, say). Under reduced motion the
 * row doesn't move; the action still happens.
 */
import { Heart, HeartOff, ListEnd } from "lucide-react";
import { useCallback, useEffect, useRef, useState, type CSSProperties, type MouseEvent as ReactMouseEvent, type PointerEvent as ReactPointerEvent, type ReactNode } from "react";
import type { Track } from "../lib/types";
import { isLiked, setLiked, useIsLiked } from "../stores/likes";
import { player } from "../stores/player";
import { tap } from "./bits";
import { lockAxis, type Axis } from "./gestures";
import { EDGE_PX, reduced } from "./motion";

/** Letting go this far either way runs that side's action. */
export const SWIPE_MARK_PX = 72;
/** The older name: right swipes add to the queue at this distance. */
export const QUEUE_SWIPE_PX = SWIPE_MARK_PX;
/** Past the mark the row follows at this share of the finger. */
const PAST_GIVE = 1 / 3;
/** And never further than this. */
export const SWIPE_MAX_PX = 128;

export type SwipeSide = "right" | "left";

/** Where the row sits for a finger `dx` (positive right), given the sides that act. */
export function swipeFollow(dx: number, sides: { right?: boolean; left?: boolean } = { right: true }): number {
  if (dx === 0 || (dx > 0 && !sides.right) || (dx < 0 && !sides.left)) return 0;
  const d = Math.abs(dx);
  const at = d <= SWIPE_MARK_PX ? d : Math.min(SWIPE_MAX_PX, SWIPE_MARK_PX + (d - SWIPE_MARK_PX) * PAST_GIVE);
  return Math.sign(dx) * at;
}

/** Which side letting go would act on: sideways and far enough, or none. */
export function swipeSide(axis: Axis | null, dx: number): SwipeSide | null {
  if (axis !== "x" || Math.abs(dx) < SWIPE_MARK_PX) return null;
  return dx > 0 ? "right" : "left";
}

/** Whether letting go adds the song: sideways, rightwards, far enough. */
export function swipeAdds(axis: Axis | null, dx: number): boolean {
  return swipeSide(axis, dx) === "right";
}

/**
 * Once a row has a sideways swipe, the page mustn't scroll under the finger
 * (WebKit can start a scroll on a slightly slanted move and cancel ours).
 * React's touch handlers are passive, so one blocking touchmove listener is
 * added by hand per list (not per row), before any touch starts, and it only
 * acts while a swipe is sideways; an up or down press passes untouched.
 */
let sideways = false;
const holders = new WeakMap<Element, number>();
function holdScroll(e: TouchEvent) {
  if (sideways && e.cancelable) e.preventDefault();
}
function holdList(el: Element) {
  const n = holders.get(el) ?? 0;
  if (n === 0) el.addEventListener("touchmove", holdScroll as EventListener, { passive: false });
  holders.set(el, n + 1);
}
function releaseList(el: Element) {
  const n = (holders.get(el) ?? 1) - 1;
  if (n <= 0) {
    el.removeEventListener("touchmove", holdScroll as EventListener);
    holders.delete(el);
  } else holders.set(el, n);
}

interface Press {
  id: number;
  x0: number;
  y0: number;
  axis: Axis | null;
  dx: number;
}

export interface SwipeActions {
  /** Swiping right (the song row's: add to the queue). */
  right?: () => void;
  /** Swiping left (like or unlike; in the queue, remove). */
  left?: () => void;
}

/**
 * The row's handlers, ref, class and style. Each action runs once per swipe
 * that gets past the mark on its side.
 */
export function useRowSwipe(actions: SwipeActions, enabled = true) {
  const press = useRef<Press | null>(null);
  const swiped = useRef(false);
  const live = useRef(actions);
  live.current = actions;
  const [offset, setOffset] = useState(0);
  const [dragging, setDragging] = useState(false);
  const [armed, setArmed] = useState<SwipeSide | null>(null);
  const sides = { right: !!actions.right, left: !!actions.left };

  // The row's list holds the page still while this row swipes (see holdList).
  const list = useRef<Element | null>(null);
  const ref = useCallback((el: HTMLElement | null) => {
    if (list.current) releaseList(list.current);
    list.current = el?.parentElement ?? null;
    if (list.current) holdList(list.current);
  }, []);
  // A row that goes away mid-swipe doesn't leave the lists holding the page.
  useEffect(
    () => () => {
      if (press.current?.axis === "x") sideways = false;
    },
    [],
  );
  /** Let go of the press without acting. */
  const drop = () => {
    if (press.current?.axis === "x") sideways = false;
    press.current = null;
  };

  /** Back to rest: no press, the row home, the mark out. */
  const reset = () => {
    drop();
    setDragging(false);
    setOffset(0);
    setArmed(null);
  };
  // The swipe's own click is swallowed; anything after it (a tap, Enter) gets through.
  const forgetSwipe = () => {
    setTimeout(() => {
      swiped.current = false;
    }, 0);
  };
  /** Only sides with an action count. */
  const acting = (axis: Axis | null, dx: number): SwipeSide | null => {
    const side = swipeSide(axis, dx);
    return side && live.current[side] ? side : null;
  };

  const onPointerDown = (e: ReactPointerEvent<HTMLElement>) => {
    // One finger at a time: a second one while a press is live is ignored.
    if (press.current) return;
    swiped.current = false;
    if (!enabled || (!sides.right && !sides.left) || (e.pointerType === "mouse" && e.button !== 0)) return;
    // The left edge belongs to the back swipe.
    const left = e.currentTarget.closest(".phone")?.getBoundingClientRect().left ?? 0;
    if (e.clientX - left <= EDGE_PX) return;
    press.current = { id: e.pointerId, x0: e.clientX, y0: e.clientY, axis: null, dx: 0 };
  };
  const onPointerMove = (e: ReactPointerEvent<HTMLElement>) => {
    const p = press.current;
    if (!p || p.id !== e.pointerId) return;
    const dx = e.clientX - p.x0;
    const dy = e.clientY - p.y0;
    if (!p.axis) {
      const axis = lockAxis(dx, dy);
      if (!axis) return;
      // Up or down first, or a way this row doesn't act: a scroll (or a shelf), not ours.
      if (axis === "y" || (dx > 0 && !live.current.right) || (dx < 0 && !live.current.left)) {
        drop();
        return;
      }
      p.axis = axis;
      sideways = true;
      swiped.current = true;
      e.currentTarget.setPointerCapture?.(e.pointerId);
      setDragging(true);
    }
    const was = acting(p.axis, p.dx);
    p.dx = dx;
    const now = acting(p.axis, dx);
    // A light tick as a mark lights: letting go now acts.
    if (now && now !== was) tap("light");
    if (now !== was) setArmed(now);
    if (!reduced()) setOffset(swipeFollow(dx, { right: !!live.current.right, left: !!live.current.left }));
  };
  const end = (e: ReactPointerEvent<HTMLElement>, cancelled: boolean) => {
    const p = press.current;
    if (!p || p.id !== e.pointerId) return;
    reset();
    if (swiped.current) forgetSwipe();
    const side = cancelled ? null : acting(p.axis, p.dx);
    if (side) {
      // Felt even right after the tick at the mark.
      tap("success", true);
      live.current[side]?.();
    }
  };
  // A swipe never also taps the row it started on.
  const onClickCapture = (e: ReactMouseEvent<HTMLElement>) => {
    if (!swiped.current) return;
    swiped.current = false;
    e.preventDefault();
    e.stopPropagation();
  };
  const reach = armed ? 1 : Math.min(1, Math.abs(offset) / SWIPE_MARK_PX);
  const way = offset > 0 || armed === "right" ? "is-right" : offset < 0 || armed === "left" ? "is-left" : "";
  return {
    ref,
    props: {
      onPointerDown,
      onPointerMove,
      onPointerUp: (e: ReactPointerEvent<HTMLElement>) => end(e, false),
      onPointerCancel: (e: ReactPointerEvent<HTMLElement>) => end(e, true),
      // Capture taken away (the system took the touch): the row never stays out.
      onLostPointerCapture: (e: ReactPointerEvent<HTMLElement>) => end(e, true),
      // A mouse press that leaves the row before it picked a way would stay live
      // (no capture yet, so its release lands elsewhere): drop it.
      onPointerLeave: (e: ReactPointerEvent<HTMLElement>) => {
        const p = press.current;
        if (p && p.id === e.pointerId && e.pointerType === "mouse" && !p.axis) drop();
      },
      onClickCapture,
      style: { "--ph-swipe": `${offset}px`, "--ph-swipe-reach": reach } as CSSProperties,
    },
    className: `ph-swipe ${dragging ? "is-swiping" : ""} ${armed ? `is-armed is-armed-${armed}` : ""} ${way}`,
    offset,
    armed,
  };
}

/** A song row's swipes: right adds it to the queue, left likes it (or unlikes a liked one). */
export function useSongSwipe(track: Track, { queue = true }: { queue?: boolean } = {}) {
  return useRowSwipe({
    right: queue ? () => void player.addToQueue([track]) : undefined,
    left: () => void setLiked([track], !isLiked(track.id)),
  });
}

/** Swipe right to queue only (rows that don't like). */
export function useSwipeToQueue(onAdd: () => void, enabled = true) {
  return useRowSwipe({ right: onAdd }, enabled);
}

/**
 * The marks under a row while it is swiped: the right swipe's at the left
 * (what the row uncovers), the left swipe's at the right.
 */
export function SwipeUnder({ right = <ListEnd size={22} />, left }: { right?: ReactNode | false; left?: ReactNode }) {
  return (
    <>
      {right !== false && (
        <span className="ph-swipe__under ph-swipe__under--right" aria-hidden="true">
          {right}
        </span>
      )}
      {left && (
        <span className="ph-swipe__under ph-swipe__under--left" aria-hidden="true">
          {left}
        </span>
      )}
    </>
  );
}

/** A song row's marks: the queue on one side, the heart on the other (broken when it would unlike). */
export function SongSwipeUnder({ trackId, queue = true }: { trackId: string; queue?: boolean }) {
  const liked = useIsLiked(trackId);
  return <SwipeUnder right={queue ? undefined : false} left={liked ? <HeartOff size={22} /> : <Heart size={22} />} />;
}
