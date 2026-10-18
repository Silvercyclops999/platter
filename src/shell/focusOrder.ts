/**
 * Tab order through the shell: the skip link, the top bar, the sidebar, the
 * player bar, then the page (and the right panel after it). The player bar
 * sits at the bottom of the window but comes before the page, so the
 * playback controls are a few Tabs away from anywhere. Lists and chip rows
 * are one stop each (they keep a single tabindex="0" child).
 */
import { useEffect, type RefObject } from "react";

/** Regions in Tab order. Anything not in one keeps the browser's own order. */
export const FOCUS_REGIONS = [
  ".skip-link",
  ".topbar, .frame-bar",
  // Deck's faceplate and Turntable's column are players drawn as the
  // frame's first object, so they keep their place in reading order.
  ".sidebar, .resize-handle--sidebar, .frame-left, .frame-edge__row, .deck-face, .deck-rail, .tt-column, .tt-browse-head",
  ".remote-strip, .player-bar, .transport-strip",
  ".frame-page__search, .main, .frame-page__panel",
  ".resize-handle--right, .queue-panel, .np-panel, .output-panel, .right-strip, .frame-right, .frame-drawer",
] as const;

const TABBABLE = 'a[href], button, input, select, textarea, [tabindex], [contenteditable="true"]';

function isTabbable(el: HTMLElement): boolean {
  if (el.tabIndex < 0) return false;
  if ((el as HTMLButtonElement).disabled) return false;
  if (el.closest("[inert], [hidden], [aria-hidden='true']")) return false;
  if (el instanceof HTMLInputElement && el.type === "hidden") return false;
  // Not laid out (display: none, or a collapsed parent). jsdom has no layout, so it counts everything.
  if (typeof el.checkVisibility === "function" && !el.checkVisibility({ visibilityProperty: true } as CheckVisibilityOptions)) return false;
  return true;
}

/** Which region an element is in, or -1. */
export function regionOf(el: Element, root: ParentNode = document): number {
  for (let i = 0; i < FOCUS_REGIONS.length; i++) {
    const hit = el.closest(FOCUS_REGIONS[i]);
    if (hit && (root === document || (root as Element).contains(hit))) return i;
  }
  return -1;
}

/** The shell's tab stops in region order (document order inside each region). */
export function shellTabOrder(root: HTMLElement): HTMLElement[] {
  const all = Array.from(root.querySelectorAll<HTMLElement>(TABBABLE)).filter(isTabbable);
  const ranked = all.map((el, i) => ({ el, i, r: regionOf(el, root) })).filter((x) => x.r >= 0);
  ranked.sort((a, b) => a.r - b.r || a.i - b.i);
  return ranked.map((x) => x.el);
}

/**
 * The element Tab (or Shift+Tab) should move to from `from`, or null to
 * leave it to the browser (outside the shell's regions, or past either end).
 */
export function nextTabStop(root: HTMLElement, from: HTMLElement, back: boolean): HTMLElement | null {
  if (regionOf(from, root) < 0) return null;
  const order = shellTabOrder(root);
  const at = order.indexOf(from);
  if (at < 0) {
    // Focus is on something that isn't a stop itself (a roving list's row, a scroller):
    // step from the stop that contains it, or the region's first.
    const holder = order.find((el) => el.contains(from));
    if (!holder) return null;
    return order[order.indexOf(holder) + (back ? -1 : 1)] ?? null;
  }
  return order[at + (back ? -1 : 1)] ?? null;
}

/** Applies the region Tab order inside `root` (the shell). Modal layers keep their own trap. */
export function useShellTabOrder(root: RefObject<HTMLElement | null>) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Tab" || e.ctrlKey || e.metaKey || e.altKey || e.defaultPrevented) return;
      const shell = root.current;
      const from = document.activeElement;
      if (!shell || !(from instanceof HTMLElement) || !shell.contains(from)) return;
      const next = nextTabStop(shell, from, e.shiftKey);
      if (!next) return;
      e.preventDefault();
      next.focus();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [root]);
}

/** The skip link's target: the page's first stop, else the main region itself. */
export function skipToMain() {
  const main = document.querySelector<HTMLElement>(".main");
  if (!main) return;
  const first = Array.from(main.querySelectorAll<HTMLElement>(TABBABLE)).find(isTabbable);
  if (first) first.focus();
  else {
    main.tabIndex = -1;
    main.focus();
  }
}
