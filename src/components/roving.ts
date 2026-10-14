/**
 * Roving tab stop for a row of buttons (filter chips): the whole row is one
 * Tab stop, and Left / Right (Home / End) move between its buttons. The stop
 * is the focused button, else the pressed one, else the first.
 */
import { useLayoutEffect, useRef, type FocusEvent, type KeyboardEvent, type RefObject } from "react";

const ITEMS = "button:not([disabled])";

function items(root: HTMLElement | null): HTMLElement[] {
  return Array.from(root?.querySelectorAll<HTMLElement>(ITEMS) ?? []);
}

function setStop(all: HTMLElement[], stop: HTMLElement | undefined) {
  for (const el of all) el.tabIndex = el === stop ? 0 : -1;
}

/** Puts the tab stop on the right button after every render. */
export function placeStop(root: HTMLElement | null) {
  const all = items(root);
  if (!all.length) return;
  const active = document.activeElement;
  const stop =
    all.find((el) => el === active) ??
    all.find((el) => el.getAttribute("aria-pressed") === "true" || el.getAttribute("aria-selected") === "true") ??
    all[0];
  setStop(all, stop);
}

/** Arrow keys inside a roving row. Returns true when the key was used. */
export function rovingKey(root: HTMLElement | null, e: Pick<KeyboardEvent, "key" | "target" | "preventDefault">): boolean {
  const all = items(root);
  const at = all.indexOf(e.target as HTMLElement);
  if (at < 0) return false;
  let next = -1;
  if (e.key === "ArrowRight") next = Math.min(all.length - 1, at + 1);
  else if (e.key === "ArrowLeft") next = Math.max(0, at - 1);
  else if (e.key === "Home") next = 0;
  else if (e.key === "End") next = all.length - 1;
  if (next < 0) return false;
  e.preventDefault();
  setStop(all, all[next]);
  all[next].focus();
  all[next].scrollIntoView?.({ block: "nearest", inline: "nearest" });
  return true;
}

/** Props and ref for a roving row of buttons. */
export function useRovingRow<T extends HTMLElement = HTMLDivElement>(): {
  ref: RefObject<T | null>;
  onKeyDown: (e: KeyboardEvent<T>) => void;
  onFocus: (e: FocusEvent<T>) => void;
} {
  const ref = useRef<T>(null);
  useLayoutEffect(() => placeStop(ref.current));
  return {
    ref,
    onKeyDown: (e) => void rovingKey(ref.current, e),
    onFocus: (e) => {
      const all = items(ref.current);
      if (all.includes(e.target as HTMLElement)) setStop(all, e.target as HTMLElement);
    },
  };
}
