/**
 * Overlay-style scrollbars: a scroller's thumb fades in while it's scrolled
 * or the pointer moves over it, and hides again after just under a second
 * without either. Two passive document listeners serve every scroller; the
 * fade itself is CSS (`--sb-alpha` in global.css).
 */

/** Idle time before the thumb hides. */
export const SCROLLBAR_IDLE_MS = 900;
/** Pointer moves closer together than this don't re-check the scroller. */
const MOVE_THROTTLE_MS = 120;

const timers = new WeakMap<Element, ReturnType<typeof setTimeout>>();

/** Show `el`'s scrollbar and (re)start its idle timer. */
export function showScrollbar(el: Element, idleMs = SCROLLBAR_IDLE_MS): void {
  // `sb` carries the transition, so the fade-out animates too.
  if (!el.classList.contains("sb")) el.classList.add("sb");
  el.classList.add("sb-on");
  const t = timers.get(el);
  if (t) clearTimeout(t);
  timers.set(
    el,
    setTimeout(() => {
      timers.delete(el);
      el.classList.remove("sb-on");
    }, idleMs),
  );
}

/** Nearest ancestor (or self) that scrolls vertically and has something to scroll. */
export function scrollerFor(start: Element | null, win: Window = window): Element | null {
  for (let el = start; el && el !== win.document.documentElement; el = el.parentElement) {
    if (el.scrollHeight <= el.clientHeight + 1) continue;
    const oy = win.getComputedStyle(el).overflowY;
    if (oy === "auto" || oy === "scroll") return el;
  }
  return null;
}

/** Start watching scroll and pointer movement. Returns an uninstall function. */
export function installOverlayScrollbars(win: Window = window): () => void {
  const doc = win.document;
  let lastMove = 0;
  const onScroll = (e: Event) => {
    if (e.target instanceof Element) showScrollbar(e.target);
  };
  const onMove = (e: PointerEvent) => {
    if (e.pointerType === "touch" || e.timeStamp - lastMove < MOVE_THROTTLE_MS) return;
    lastMove = e.timeStamp;
    const el = scrollerFor(e.target instanceof Element ? e.target : null, win);
    if (el) showScrollbar(el);
  };
  doc.addEventListener("scroll", onScroll, { capture: true, passive: true });
  doc.addEventListener("pointermove", onMove, { passive: true });
  return () => {
    doc.removeEventListener("scroll", onScroll, { capture: true });
    doc.removeEventListener("pointermove", onMove);
  };
}
