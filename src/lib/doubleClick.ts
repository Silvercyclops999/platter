import { useRef, type MouseEvent } from "react";

/** Longer than any system double-click interval. */
export const DOUBLE_CLICK_WINDOW_MS = 1500;

/**
 * Tells a real double-click on an element from one whose first click landed
 * somewhere else. Clicking a card opens its page at once, so the second
 * click of a double-click on the card can land on a song row of the new
 * page, and the browser reports a double-click there. Pass `onMouseDown` to
 * the element and check `stray()` at the top of its double-click handler.
 */
export function useOwnDoubleClick(): {
  onMouseDown: (e: MouseEvent) => void;
  stray: () => boolean;
} {
  const firstAt = useRef(Number.NEGATIVE_INFINITY);
  const strayDown = useRef(false);
  return {
    onMouseDown: (e) => {
      if (e.detail <= 1) {
        firstAt.current = e.timeStamp;
        strayDown.current = false;
      } else if (e.timeStamp - firstAt.current > DOUBLE_CLICK_WINDOW_MS) {
        // A second click with no first one here.
        strayDown.current = true;
      }
    },
    stray: () => {
      const s = strayDown.current;
      strayDown.current = false;
      return s;
    },
  };
}
