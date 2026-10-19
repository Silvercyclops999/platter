/**
 * Swipe a song row right to add it to the queue: the row follows the
 * finger, a queue mark shows under it, and letting go past QUEUE_SWIPE_PX
 * adds the song (a short buzz and "Added to queue"); the row slides back
 * either way. A press that heads up or down first is a scroll and the row
 * stays put (the row only takes sideways moves, see .ph-swipe in
 * phone.css). A press at the very left edge is left to the back swipe.
 * Under reduced motion the row doesn't move; the add still happens.
 */
import { ListEnd } from "lucide-react";
import { useRef, useState, type CSSProperties, type MouseEvent as ReactMouseEvent, type PointerEvent as ReactPointerEvent } from "react";
import { tap } from "./bits";
import { lockAxis, type Axis } from "./gestures";
import { EDGE_PX, reduced } from "./motion";

/** Letting go this far right adds the song. */
export const QUEUE_SWIPE_PX = 72;
/** Past the mark the row follows at this share of the finger. */
const PAST_GIVE = 1 / 3;
/** And never further than this. */
export const SWIPE_MAX_PX = 128;

/** Where the row sits for a finger `dx` to the right (never left). */
export function swipeFollow(dx: number): number {
  if (dx <= 0) return 0;
  if (dx <= QUEUE_SWIPE_PX) return dx;
  return Math.min(SWIPE_MAX_PX, QUEUE_SWIPE_PX + (dx - QUEUE_SWIPE_PX) * PAST_GIVE);
}

/** Whether letting go adds the song: sideways, rightwards, far enough. */
export function swipeAdds(axis: Axis | null, dx: number): boolean {
  return axis === "x" && dx >= QUEUE_SWIPE_PX;
}

interface Press {
  id: number;
  x0: number;
  y0: number;
  axis: Axis | null;
  dx: number;
}

/**
 * The row's handlers, class and style. `onAdd` runs once per swipe that
 * gets past the mark.
 */
export function useSwipeToQueue(onAdd: () => void, enabled = true) {
  const press = useRef<Press | null>(null);
  const swiped = useRef(false);
  const [offset, setOffset] = useState(0);
  const [dragging, setDragging] = useState(false);

  const onPointerDown = (e: ReactPointerEvent<HTMLElement>) => {
    swiped.current = false;
    if (!enabled || (e.pointerType === "mouse" && e.button !== 0)) return;
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
      p.axis = lockAxis(dx, dy);
      if (!p.axis) return;
      // Up or down first, or leftwards: a scroll (or a shelf), not ours.
      if (p.axis === "y" || dx < 0) {
        press.current = null;
        return;
      }
      swiped.current = true;
      e.currentTarget.setPointerCapture?.(e.pointerId);
      setDragging(true);
    }
    p.dx = dx;
    if (!reduced()) setOffset(swipeFollow(dx));
  };
  const end = (e: ReactPointerEvent<HTMLElement>, cancelled: boolean) => {
    const p = press.current;
    if (!p || p.id !== e.pointerId) return;
    press.current = null;
    setDragging(false);
    setOffset(0);
    if (!cancelled && swipeAdds(p.axis, p.dx)) {
      tap("success");
      onAdd();
    }
  };
  // A swipe never also taps the row it started on.
  const onClickCapture = (e: ReactMouseEvent<HTMLElement>) => {
    if (!swiped.current) return;
    swiped.current = false;
    e.preventDefault();
    e.stopPropagation();
  };
  const reach = Math.min(1, offset / QUEUE_SWIPE_PX);
  return {
    props: {
      onPointerDown,
      onPointerMove,
      onPointerUp: (e: ReactPointerEvent<HTMLElement>) => end(e, false),
      onPointerCancel: (e: ReactPointerEvent<HTMLElement>) => end(e, true),
      onClickCapture,
      style: { "--ph-swipe": `${offset}px`, "--ph-swipe-reach": reach } as CSSProperties,
    },
    className: `ph-swipe ${dragging ? "is-swiping" : ""} ${offset >= QUEUE_SWIPE_PX ? "is-armed" : ""}`,
    offset,
  };
}

/** The mark under a row while it is swiped. */
export function SwipeUnder() {
  return (
    <span className="ph-swipe__under" aria-hidden="true">
      <ListEnd size={22} />
    </span>
  );
}
