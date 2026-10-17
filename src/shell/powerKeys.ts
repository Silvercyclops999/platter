/**
 * Keyboard power mode (Settings, off by default). With it on, in the main
 * window:
 *
 * - the hint key (F unless changed) puts a two-letter label next to every
 *   clickable thing in view; typing a label activates that thing, Esc or any
 *   other key hides them;
 * - j / k move the row selection in the track list (down / up), g g jumps to
 *   the top and G to the bottom, Enter plays the selected row;
 * - / focuses search.
 *
 * Labels sit outside the element they name (left, else above, else below,
 * else right), so they never cover its text.
 */
import { create } from "zustand";

/** Home row first, then the rows above and below. */
export const HINT_ALPHABET = "asdfjklghqwertyuiopzxcvbnm";

/** `n` distinct two-letter labels, home row first. */
export function hintLabels(n: number, alphabet = HINT_ALPHABET): string[] {
  const out: string[] = [];
  for (const a of alphabet) {
    for (const b of alphabet) {
      if (out.length >= n) return out;
      out.push(a + b);
    }
  }
  return out;
}

export interface Box {
  left: number;
  top: number;
  width: number;
  height: number;
}

/** Gap between a label and what it names. */
const GAP = 2;

/** Whether two boxes overlap (touching edges don't). */
export function overlaps(a: Box, b: Box): boolean {
  return a.left < b.left + b.width && b.left < a.left + a.width && a.top < b.top + b.height && b.top < a.top + a.height;
}

/**
 * Where a label of `size` goes for a target: to its left, else above, else
 * below, else to its right, the first that fits in the viewport. Never on
 * the target itself: null when there's no room around it (it fills the
 * window), and it gets no hint.
 */
export function placeHint(
  target: Box,
  size: { width: number; height: number },
  viewport: { width: number; height: number },
): { left: number; top: number } | null {
  const midY = target.top + (target.height - size.height) / 2;
  const tries = [
    { left: target.left - size.width - GAP, top: midY },
    { left: target.left, top: target.top - size.height - GAP },
    { left: target.left, top: target.top + target.height + GAP },
    { left: target.left + target.width + GAP, top: midY },
  ];
  const fits = (p: { left: number; top: number }) =>
    p.left >= 0 && p.top >= 0 && p.left + size.width <= viewport.width && p.top + size.height <= viewport.height;
  return tries.find(fits) ?? null;
}

/** Things a hint can activate. */
export const CLICKABLE = [
  "button:not([disabled])",
  "a[href]",
  "input:not([disabled]):not([type=hidden])",
  "select:not([disabled])",
  "textarea:not([disabled])",
  "[role=button]:not([aria-disabled=true])",
  "[role=link]",
  "[role=tab]",
  "[role=switch]",
  "[role=checkbox]",
  "[role=option]",
  "[role=menuitem]",
  "[role=menuitemradio]",
  "[role=menuitemcheckbox]",
  ".track-row[data-row-key]",
].join(", ");

export interface Hint {
  label: string;
  el: HTMLElement;
  left: number;
  top: number;
}

/** Approximate label size (two capital letters in a small key cap). */
export const HINT_SIZE = { width: 26, height: 18 };

function visible(el: HTMLElement, box: Box, vw: number, vh: number): boolean {
  if (box.width <= 0 || box.height <= 0) return false;
  if (box.left + box.width <= 0 || box.top + box.height <= 0 || box.left >= vw || box.top >= vh) return false;
  return !el.closest("[aria-hidden=true], [inert], .power-hints");
}

/**
 * The clickable things in view, labelled. Inside an open modal dialog only
 * its own controls count. `rectOf` is the layout read (tests fake it).
 */
export function collectHints(
  doc: Document = document,
  rectOf: (el: HTMLElement) => Box = (el) => el.getBoundingClientRect(),
  viewport = { width: window.innerWidth, height: window.innerHeight },
): Hint[] {
  const dialogs = Array.from(doc.querySelectorAll<HTMLElement>("[role=dialog][aria-modal=true], [role=alertdialog][aria-modal=true]"));
  const root: ParentNode = dialogs[dialogs.length - 1] ?? doc;
  const found: { el: HTMLElement; at: { left: number; top: number } }[] = [];
  for (const el of Array.from(root.querySelectorAll<HTMLElement>(CLICKABLE))) {
    const box = rectOf(el);
    const at = visible(el, box, viewport.width, viewport.height) ? placeHint(box, HINT_SIZE, viewport) : null;
    if (at) found.push({ el, at });
  }
  const labels = hintLabels(found.length);
  return found.slice(0, labels.length).map(({ el, at }, i) => ({ label: labels[i], el, ...at }));
}

/** Use what a hint names: fields get focus, everything else a click. */
export function activate(el: HTMLElement) {
  if (el.matches("input, textarea, select, [contenteditable=true]")) {
    el.focus();
    return;
  }
  el.focus?.({ preventScroll: true });
  el.click();
}

// ---------------------------------------------------------------- state

interface PowerState {
  hints: Hint[] | null;
  typed: string;
}

export const usePowerHints = create<PowerState>()(() => ({ hints: null, typed: "" }));

export function showHints(hints = collectHints()) {
  usePowerHints.setState({ hints: hints.length ? hints : null, typed: "" });
}

export function hideHints() {
  if (usePowerHints.getState().hints) usePowerHints.setState({ hints: null, typed: "" });
}

/** A letter typed while hints show: narrow them, activate an exact match, hide on a miss. */
export function typeHint(letter: string) {
  const { hints, typed } = usePowerHints.getState();
  if (!hints) return;
  const next = typed + letter.toLowerCase();
  const left = hints.filter((h) => h.label.startsWith(next));
  const hit = left.find((h) => h.label === next);
  if (hit) {
    hideHints();
    activate(hit.el);
  } else if (!left.length) hideHints();
  else usePowerHints.setState({ typed: next });
}

// ---------------------------------------------------------------- rows

const ROW = ".track-row[data-row-key]";

function focusedRow(): HTMLElement | null {
  const a = document.activeElement as HTMLElement | null;
  return a?.closest?.<HTMLElement>(ROW) ?? null;
}

/** The track list j / k start in: the one inside an open dialog, else the first on the page. */
function firstList(): HTMLElement | null {
  const lists = Array.from(document.querySelectorAll<HTMLElement>("[data-tracklist]")).filter((l) => l.querySelector(ROW));
  const inDialog = lists.find((l) => l.closest("[role=dialog]"));
  return inDialog ?? lists.find((l) => l.closest("main, #main, [data-region=main]")) ?? lists[0] ?? null;
}

/**
 * Move the row selection: from the focused row as the arrow keys do, or
 * from nothing to the first (or last) row of the list in view.
 */
export function moveRow(key: "ArrowDown" | "ArrowUp" | "Home" | "End"): boolean {
  const row = focusedRow();
  if (row) {
    row.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
    return true;
  }
  const rows = Array.from(firstList()?.querySelectorAll<HTMLElement>(ROW) ?? []);
  const to = key === "End" || key === "ArrowUp" ? rows[rows.length - 1] : rows[0];
  if (!to) return false;
  to.focus();
  to.click();
  to.scrollIntoView?.({ block: "nearest" });
  return true;
}

/** Enter with nothing focused plays the selected row. */
function playSelected(): boolean {
  const row = document.querySelector<HTMLElement>(`${ROW}.is-picked`);
  if (!row) return false;
  row.focus();
  row.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
  return true;
}

/** Two g presses within this long jump to the top. */
export const GG_MS = 700;
let lastG = 0;

export function resetPowerKeys() {
  lastG = 0;
  usePowerHints.setState({ hints: null, typed: "" });
}

function isTyping(el: EventTarget | null): boolean {
  if (!(el instanceof HTMLElement)) return false;
  return el.isContentEditable || el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.tagName === "SELECT";
}

export interface PowerContext {
  hintKey: string;
  /** A menu or the command palette is open: leave keys to it. */
  busy: boolean;
  /**
   * A modal dialog is open: hints still label its controls, and j / k move in
   * its own track list ("list"), but nothing reaches the page behind it.
   */
  modal?: "list" | "no-list" | null;
  focusSearch: () => void;
  now?: number;
}

/** One key press in power mode. Returns true when it was used (the caller stops it). */
export function powerKey(e: Pick<KeyboardEvent, "key" | "altKey" | "ctrlKey" | "metaKey" | "shiftKey" | "target" | "repeat">, ctx: PowerContext): boolean {
  const plain = !e.altKey && !e.ctrlKey && !e.metaKey;
  if (usePowerHints.getState().hints) {
    if (e.key === "Escape") {
      hideHints();
      return true;
    }
    if (e.key === "Backspace") {
      const t = usePowerHints.getState().typed;
      usePowerHints.setState({ typed: t.slice(0, -1) });
      return true;
    }
    if (plain && /^[a-z]$/i.test(e.key)) {
      typeHint(e.key);
      return true;
    }
    if (e.key !== "Shift") hideHints();
    return false;
  }
  if (!plain || ctx.busy || isTyping(e.target)) return false;
  const now = ctx.now ?? Date.now();
  const isHintKey = e.key === ctx.hintKey || e.key === ctx.hintKey.toUpperCase();
  if (ctx.modal === "no-list" && !isHintKey) return false;
  if (ctx.modal && (e.key === "/" || e.key === "Enter")) return false;
  switch (e.key) {
    case ctx.hintKey:
    case ctx.hintKey.toUpperCase():
      if (e.repeat) return true;
      showHints();
      return true;
    case "j":
      return moveRow("ArrowDown");
    case "k":
      return moveRow("ArrowUp");
    case "G":
      return moveRow("End");
    case "g":
      if (now - lastG <= GG_MS) {
        lastG = 0;
        return moveRow("Home");
      }
      lastG = now;
      return true;
    case "/":
      ctx.focusSearch();
      return true;
    case "Enter":
      if (e.target && e.target !== document.body) return false;
      return playSelected();
    default:
      return false;
  }
}
