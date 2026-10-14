/**
 * Column layout for the shell: sidebar | main | right panel.
 *
 * Pure functions, so the rules can be tested without a DOM. The user's own
 * widths live in the UI store; everything here derives the widths actually
 * used for a given window size, and the widths a drag produces.
 */

/** Sidebar width range in full mode. */
export const SIDEBAR_MIN = 280;
export const SIDEBAR_MAX = 420;
export const SIDEBAR_DEFAULT = 300;
/** Width of the collapsed icon rail. */
export const RAIL_WIDTH = 72;

/** Right panel width range. It never collapses by drag. */
export const RIGHT_MIN = 280;
export const RIGHT_MAX = 420;
export const RIGHT_DEFAULT = 340;
/** Width of the strip left on the right edge when the panel is closed. */
export const STRIP_WIDTH = 24;

/** The main column never gets narrower than this while other columns can give way. */
export const MAIN_MIN = 420;
/** Window padding on each side and the gap between columns (matches shell.css). */
export const EDGE = 8;
export const GAP = 8;

/** How far past the edge (px) a drag must go to snap the sidebar shut or open. */
export const SNAP_DISTANCE = 120;
/** Keyboard step for the resize handles. */
export const KEY_STEP = 16;

export type SidebarMode = "full" | "narrowed" | "rail";

export interface LayoutInput {
  windowWidth: number;
  /** The user's sidebar width (full mode). */
  sidebarWidth: number;
  /** The user's right panel width. */
  rightWidth: number;
  rightOpen: boolean;
  /** The user collapsed the sidebar to the rail. */
  sidebarCollapsed: boolean;
}

export interface Layout {
  /** Sidebar width in use (the rail width when railed). */
  sidebar: number;
  /** full: the user's width. narrowed: auto-narrowed for space. rail: icons only. */
  sidebarMode: SidebarMode;
  /** The rail was chosen for space, not by the user. */
  sidebarAuto: boolean;
  /** Right column width in use: the panel, or the strip when closed. */
  right: number;
  main: number;
}

export const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

/** Width outside the three columns: two window edges and two gaps. */
const FRAME = 2 * EDGE + 2 * GAP;

/**
 * Share the window between the columns. With the right panel open the
 * sidebar goes to the rail first, then the right panel shrinks toward its
 * minimum. With it closed the sidebar narrows toward its minimum, then
 * rails. The main column keeps MAIN_MIN while anything else can still give
 * way. Auto states are derived, never stored, so the user's widths (both
 * panels') come back as the window widens.
 */
export function solveLayout(input: LayoutInput): Layout {
  const { windowWidth, rightOpen, sidebarCollapsed } = input;
  const userSide = clamp(input.sidebarWidth, SIDEBAR_MIN, SIDEBAR_MAX);
  let right = rightOpen ? clamp(input.rightWidth, RIGHT_MIN, RIGHT_MAX) : STRIP_WIDTH;
  let sidebar = sidebarCollapsed ? RAIL_WIDTH : userSide;
  let mode: SidebarMode = sidebarCollapsed ? "rail" : "full";
  let auto = false;

  const room = () => windowWidth - FRAME - sidebar - right - MAIN_MIN;

  // 1. The sidebar gives way first: the rail with the right panel open, narrower without.
  if (room() < 0 && mode !== "rail") {
    if (rightOpen) {
      sidebar = RAIL_WIDTH;
      mode = "rail";
      auto = true;
    } else {
      const narrowed = sidebar + room();
      if (narrowed >= SIDEBAR_MIN) {
        sidebar = narrowed;
        mode = "narrowed";
      } else {
        sidebar = RAIL_WIDTH;
        mode = "rail";
        auto = true;
      }
    }
  }

  // 2. Then the right panel, down to its minimum.
  if (room() < 0 && rightOpen) right = Math.max(RIGHT_MIN, right + room());

  const main = Math.max(0, windowWidth - FRAME - sidebar - right);
  return { sidebar, sidebarMode: mode, sidebarAuto: auto, right, main };
}

/** Widest the sidebar may be while the right column stays as it is and main keeps its floor. */
export function sidebarRoom(windowWidth: number, right: number): number {
  return Math.min(SIDEBAR_MAX, windowWidth - FRAME - right - MAIN_MIN);
}

/** Widest the right panel may be while the sidebar stays as it is and main keeps its floor. */
export function rightRoom(windowWidth: number, sidebar: number): number {
  return Math.min(RIGHT_MAX, windowWidth - FRAME - sidebar - MAIN_MIN);
}

export interface SidebarDrag {
  width: number;
  rail: boolean;
}

/**
 * Sidebar width for a drag `dx` px from where it started. The edge follows
 * the pointer between min and max (and `room`). Going SNAP_DISTANCE past the
 * minimum snaps to the rail; from the rail, dragging out that far snaps open
 * at the minimum, then follows the pointer. Stateless, so dragging back
 * across the snap point undoes it within the same drag.
 */
export function dragSidebar(start: SidebarDrag, dx: number, room = SIDEBAR_MAX): SidebarDrag {
  const max = Math.max(SIDEBAR_MIN, Math.min(SIDEBAR_MAX, room));
  if (start.rail) {
    if (dx < SNAP_DISTANCE) return { width: RAIL_WIDTH, rail: true };
    return { width: clamp(RAIL_WIDTH + dx, SIDEBAR_MIN, max), rail: false };
  }
  const raw = start.width + dx;
  if (raw <= SIDEBAR_MIN - SNAP_DISTANCE) return { width: RAIL_WIDTH, rail: true };
  return { width: clamp(raw, SIDEBAR_MIN, max), rail: false };
}

/** Right panel width for a drag `dx` px (its handle is on its left edge, so left widens). */
export function dragRight(startWidth: number, dx: number, room = RIGHT_MAX): number {
  const max = Math.max(RIGHT_MIN, Math.min(RIGHT_MAX, room));
  return clamp(startWidth - dx, RIGHT_MIN, max);
}

/**
 * Keyboard resize of the sidebar. Arrows step by KEY_STEP; Left at the
 * minimum goes to the rail and Right on the rail opens at the minimum.
 * Home and End jump to the minimum and maximum. Null for other keys.
 */
export function keySidebar(start: SidebarDrag, key: string, room = SIDEBAR_MAX): SidebarDrag | null {
  const max = Math.max(SIDEBAR_MIN, Math.min(SIDEBAR_MAX, room));
  switch (key) {
    case "ArrowLeft":
      if (start.rail) return start;
      return start.width <= SIDEBAR_MIN ? { width: RAIL_WIDTH, rail: true } : { width: Math.max(SIDEBAR_MIN, start.width - KEY_STEP), rail: false };
    case "ArrowRight":
      if (start.rail) return { width: SIDEBAR_MIN, rail: false };
      return { width: Math.min(max, start.width + KEY_STEP), rail: false };
    case "Home":
      return { width: SIDEBAR_MIN, rail: false };
    case "End":
      return { width: max, rail: false };
    default:
      return null;
  }
}

/** Keyboard resize of the right panel: Left widens, Right narrows, Home/End go to min/max. */
export function keyRight(startWidth: number, key: string, room = RIGHT_MAX): number | null {
  const max = Math.max(RIGHT_MIN, Math.min(RIGHT_MAX, room));
  switch (key) {
    case "ArrowLeft":
      return Math.min(max, startWidth + KEY_STEP);
    case "ArrowRight":
      return Math.max(RIGHT_MIN, startWidth - KEY_STEP);
    case "Home":
      return RIGHT_MIN;
    case "End":
      return max;
    default:
      return null;
  }
}
