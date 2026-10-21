/**
 * Overlay scrollbars. Native scrollbars are hidden (global.css), so no
 * scroller gives up width to a gutter. In their place a thin thumb is drawn
 * over the scroller's right edge: it fades in while the scroller is scrolled
 * or the pointer moves over it, and hides again after just under a second
 * without either. The thumb can be dragged; the wheel, keyboard and touch
 * scroll the element itself as before.
 *
 * Thumbs stack at `--z-scrollbar`, under the top bar and every menu, dialog,
 * toast and tooltip (`layer` in tokens.ts). A scroller inside one of those
 * floating layers gets its thumb one step above that layer instead.
 *
 * A scroller marked `data-scrollbar="off"` never shows one (the lyrics,
 * which scroll by themselves as the song plays).
 *
 * Two passive document listeners serve every scroller. A shown scroller also
 * gets `.sb-on` (and keeps `.sb` once seen) for styles that want to follow.
 */

import { layer } from "./tokens";

/** Idle time before the thumb hides. */
export const SCROLLBAR_IDLE_MS = 900;
/** Pointer moves closer together than this don't re-check the scroller. */
const MOVE_THROTTLE_MS = 120;
/** Thumb size and placement (CSS px). */
const THUMB_WIDTH = 8;
const THUMB_INSET = 2;
const THUMB_MIN = 24;
/** How long the fade-out runs before a hidden thumb is removed. */
const FADE_MS = 250;

const timers = new WeakMap<Element, ReturnType<typeof setTimeout>>();
const thumbs = new WeakMap<Element, HTMLElement>();
const owners = new WeakMap<Element, Element>();
const drags = new WeakSet<Element>();

/** Fallback for `--z-scrollbar` when no stylesheet is loaded. */
const Z_SCROLLBAR = layer.scrollbar;

function layerFor(doc: Document): HTMLElement {
  let layer = doc.querySelector<HTMLElement>(":scope > body > .sb-layer");
  if (!layer) {
    layer = doc.createElement("div");
    layer.className = "sb-layer";
    layer.setAttribute("aria-hidden", "true");
    doc.body.appendChild(layer);
  }
  return layer;
}

/** Thumb box for a scroller, or null when it has nothing to scroll. */
export function thumbBox(
  rect: { top: number; left: number },
  m: { clientWidth: number; clientHeight: number; scrollHeight: number; scrollTop: number; clientLeft?: number },
): { top: number; left: number; height: number } | null {
  const range = m.scrollHeight - m.clientHeight;
  if (range <= 1 || m.clientHeight <= 0) return null;
  const track = m.clientHeight - THUMB_INSET * 2;
  const height = Math.max(THUMB_MIN, Math.round((track * m.clientHeight) / m.scrollHeight));
  const at = Math.min(1, Math.max(0, m.scrollTop / range));
  return {
    top: rect.top + THUMB_INSET + Math.round((track - height) * at),
    left: rect.left + (m.clientLeft ?? 0) + m.clientWidth - THUMB_WIDTH - THUMB_INSET,
    height,
  };
}

/** Put `el`'s thumb where its scroll position says. */
function place(el: Element, thumb: HTMLElement): void {
  const box = thumbBox(el.getBoundingClientRect(), el as HTMLElement);
  if (!box) {
    thumb.style.display = "none";
    return;
  }
  if (covered(el, box)) {
    thumb.style.display = "none";
    return;
  }
  thumb.style.display = "";
  thumb.style.top = `${box.top}px`;
  thumb.style.left = `${box.left}px`;
  thumb.style.height = `${box.height}px`;
}

/** Whether something else (a menu, dialog or popover) is over the spot the
 * thumb would take. The stacking order already keeps thumbs under menus;
 * this also hides a thumb under a popover drawn inside its own scroller. */
export function covered(el: Element, box: { top: number; left: number; height: number }): boolean {
  const doc = el.ownerDocument;
  if (typeof doc.elementsFromPoint !== "function") return false;
  const x = box.left + 4;
  for (const y of [box.top + 2, box.top + box.height / 2, box.top + box.height - 2]) {
    const top = doc.elementsFromPoint(x, y).find((n) => !n.classList.contains("sb-thumb") && !n.classList.contains("sb-layer"));
    if (top && top !== el && !el.contains(top)) return true;
  }
  return false;
}

/**
 * The z-index for `el`'s thumb: `--z-scrollbar`, or one above the outermost
 * floating layer `el` sits in (a dialog, a menu, the full window view), so
 * the thumb shows on that layer and stays under anything opened over it.
 */
export function thumbZ(el: Element): number {
  const doc = el.ownerDocument;
  const win = doc.defaultView;
  if (!win) return Z_SCROLLBAR;
  const base = parseInt(win.getComputedStyle(doc.documentElement).getPropertyValue("--z-scrollbar"), 10);
  const z = Number.isFinite(base) ? base : Z_SCROLLBAR;
  let outer: number | null = null;
  for (let n: Element | null = el; n && n !== doc.documentElement; n = n.parentElement) {
    const cs = win.getComputedStyle(n);
    const own = parseInt(cs.zIndex, 10);
    if (Number.isFinite(own) && cs.position !== "static") outer = own;
  }
  return outer !== null && outer >= z ? outer + 1 : z;
}

function thumbFor(el: Element): HTMLElement {
  let thumb = thumbs.get(el);
  if (thumb?.isConnected) return thumb;
  const doc = el.ownerDocument;
  thumb = doc.createElement("div");
  thumb.className = "sb-thumb";
  thumb.style.zIndex = String(thumbZ(el));
  thumbs.set(el, thumb);
  owners.set(thumb, el);
  thumb.addEventListener("pointerdown", (e) => startDrag(el, thumb!, e));
  layerFor(doc).appendChild(thumb);
  return thumb;
}

/** Dragging the thumb scrolls its element in proportion. */
function startDrag(el: Element, thumb: HTMLElement, e: PointerEvent): void {
  if (e.button !== 0) return;
  e.preventDefault();
  const s = el as HTMLElement;
  const startY = e.clientY;
  const startTop = s.scrollTop;
  const track = s.clientHeight - THUMB_INSET * 2 - thumb.offsetHeight;
  const ratio = track > 0 ? (s.scrollHeight - s.clientHeight) / track : 0;
  drags.add(el);
  thumb.classList.add("is-dragging");
  thumb.setPointerCapture?.(e.pointerId);
  const move = (ev: PointerEvent) => {
    s.scrollTop = startTop + (ev.clientY - startY) * ratio;
    showScrollbar(el);
  };
  const end = () => {
    drags.delete(el);
    thumb.classList.remove("is-dragging");
    thumb.removeEventListener("pointermove", move);
    thumb.removeEventListener("pointerup", end);
    thumb.removeEventListener("pointercancel", end);
    showScrollbar(el);
  };
  thumb.addEventListener("pointermove", move);
  thumb.addEventListener("pointerup", end);
  thumb.addEventListener("pointercancel", end);
}

/** Show `el`'s scrollbar and (re)start its idle timer. */
export function showScrollbar(el: Element, idleMs = SCROLLBAR_IDLE_MS): void {
  if (el.getAttribute("data-scrollbar") === "off") return;
  if (!el.classList.contains("sb")) el.classList.add("sb");
  el.classList.add("sb-on");
  if (el.ownerDocument?.body) {
    const thumb = thumbFor(el);
    place(el, thumb);
    thumb.classList.add("is-on");
  }
  const t = timers.get(el);
  if (t) clearTimeout(t);
  timers.set(
    el,
    setTimeout(function hide() {
      // Never hide under a drag; check again later.
      if (drags.has(el)) {
        timers.set(el, setTimeout(hide, idleMs));
        return;
      }
      timers.delete(el);
      el.classList.remove("sb-on");
      const thumb = thumbs.get(el);
      if (!thumb) return;
      thumb.classList.remove("is-on");
      setTimeout(() => {
        if (!thumb.classList.contains("is-on")) {
          thumb.remove();
          thumbs.delete(el);
        }
      }, FADE_MS);
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
    const target = e.target instanceof Element ? e.target : null;
    // Pointing at a thumb keeps its scroller's bar up.
    const owner = target ? owners.get(target) : undefined;
    const el = owner ?? scrollerFor(target, win);
    if (el) showScrollbar(el);
  };
  doc.addEventListener("scroll", onScroll, { capture: true, passive: true });
  doc.addEventListener("pointermove", onMove, { passive: true });
  return () => {
    doc.removeEventListener("scroll", onScroll, { capture: true });
    doc.removeEventListener("pointermove", onMove);
    doc.querySelector(":scope > body > .sb-layer")?.remove();
  };
}
