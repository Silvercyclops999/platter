/**
 * The Console layout's table and panes, as numbers and rules: the columns
 * it can show, how a sort is written, the pane width limits (the core keeps
 * the same ones) and how the window is shared out. Pure, so it can be
 * tested without a DOM.
 */
import { isHiRes, qualityLabel } from "./format";
import type { Track } from "./types";

/** Columns after the name, which is always shown (with the small cover). */
export type ConsoleColumn = "artist" | "album" | "year" | "format" | "length" | "plays";
export const CONSOLE_COLUMNS: readonly ConsoleColumn[] = ["artist", "album", "year", "format", "length", "plays"];
export const CONSOLE_DEFAULT_COLUMNS: ConsoleColumn[] = ["artist", "album", "year", "format", "length"];

export const COLUMN_NAME: Record<ConsoleColumn | "name", string> = {
  name: "Name",
  artist: "Artist",
  album: "Album",
  year: "Year",
  format: "Format",
  length: "Length",
  plays: "Plays",
};

export const PANE_LIMITS = {
  tree: { min: 180, max: 400, start: 240 },
  details: { min: 240, max: 440, start: 300 },
} as const;
export type Pane = keyof typeof PANE_LIMITS;

export function clampPane(pane: Pane, px: number): number {
  const l = PANE_LIMITS[pane];
  if (!Number.isFinite(px)) return l.start;
  return Math.round(Math.min(l.max, Math.max(l.min, px)));
}

/** A sort is "" (the list's own order), a column, or "-" and a column for descending. */
export type SortKey = ConsoleColumn | "name";
export function consoleSortOk(s: string): boolean {
  const col = s.startsWith("-") ? s.slice(1) : s;
  return s === "" || col === "name" || (CONSOLE_COLUMNS as readonly string[]).includes(col);
}

export function parseSort(s: string | null | undefined): { key: SortKey; desc: boolean } | null {
  if (!s || !consoleSortOk(s)) return null;
  const desc = s.startsWith("-");
  return { key: (desc ? s.slice(1) : s) as SortKey, desc };
}

/** A header click: ascending, then descending, then back to the list's own order. */
export function nextSort(current: string, key: SortKey): string {
  const p = parseSort(current);
  if (!p || p.key !== key) return key;
  return p.desc ? "" : `-${key}`;
}

/** One row of the table: the song, and the year when the source knows it. */
export interface ConsoleRow {
  track: Track;
  year: number | null;
  /** Place in the source, for its own order. */
  index: number;
}

export function formatCell(t: Track): string {
  return qualityLabel(t) ?? (t.codec ? t.codec.toUpperCase() : "");
}

function value(r: ConsoleRow, key: SortKey): string | number | null {
  const t = r.track;
  switch (key) {
    case "name":
      return t.title;
    case "artist":
      return t.trackArtist || t.artistTitle;
    case "album":
      return t.albumTitle;
    case "year":
      return r.year;
    case "format":
      // Higher resolution sorts after lower: depth, then rate, then bitrate.
      return (t.bitDepth ?? 0) * 1e9 + (t.sampleRate ?? 0) * 10 + (t.bitrate ?? 0) / 1e4;
    case "length":
      return t.durationMs;
    case "plays":
      return t.viewCount;
  }
}

const collator = new Intl.Collator(undefined, { sensitivity: "base", numeric: true });

/** Rows in the sort's order; empty values go last either way, ties keep the list's order. */
export function sortRows(rows: ConsoleRow[], sort: string): ConsoleRow[] {
  const p = parseSort(sort);
  if (!p) return [...rows].sort((a, b) => a.index - b.index);
  const dir = p.desc ? -1 : 1;
  return [...rows].sort((a, b) => {
    const x = value(a, p.key);
    const y = value(b, p.key);
    const xe = x === null || x === "";
    const ye = y === null || y === "";
    if (xe || ye) return xe && ye ? a.index - b.index : xe ? 1 : -1;
    const c = typeof x === "number" && typeof y === "number" ? x - y : collator.compare(String(x), String(y));
    return c ? c * dir : a.index - b.index;
  });
}

/** The table's quick filters (the tree's Filters group). */
export type ConsoleFilter = "lossless" | "hiRes" | "unplayed";
export const FILTER_NAME: Record<ConsoleFilter, string> = { lossless: "Lossless", hiRes: "Hi-res", unplayed: "Never played" };

const LOSSLESS = new Set(["flac", "alac", "wav", "aiff", "ape", "wv", "dsd", "dsf", "dff", "pcm"]);
export function isLossless(t: Track): boolean {
  return !!t.codec && LOSSLESS.has(t.codec.toLowerCase());
}

export function filterRows(rows: ConsoleRow[], filters: readonly ConsoleFilter[]): ConsoleRow[] {
  if (!filters.length) return rows;
  return rows.filter(({ track: t }) =>
    filters.every((f) => (f === "lossless" ? isLossless(t) : f === "hiRes" ? isHiRes(t) : t.viewCount === 0)),
  );
}

/** Edges, the two resize handles and the gaps between panes (px, matches console.css). */
const CONSOLE_EDGES = 16 + 2 * 12;
/** The table keeps at least this width while the panes give way. */
export const CONSOLE_TABLE_MIN = 520;
export const CONSOLE_RAIL = 64;

export interface ConsoleSolve {
  tree: number;
  treeMode: "full" | "rail";
  /** The details pane as a column beside the table, or a drawer over it. */
  detailsMode: "column" | "drawer";
  details: number;
}

/**
 * Share the window: the details pane becomes a drawer first, then the tree
 * folds to an icon rail, so the table keeps a usable width.
 */
export function solveConsole(width: number, tree: number, details: number): ConsoleSolve {
  const t = clampPane("tree", tree);
  const d = clampPane("details", details);
  if (width - CONSOLE_EDGES - t - d >= CONSOLE_TABLE_MIN) return { tree: t, treeMode: "full", detailsMode: "column", details: d };
  if (width - CONSOLE_EDGES - t >= CONSOLE_TABLE_MIN) return { tree: t, treeMode: "full", detailsMode: "drawer", details: d };
  return { tree: CONSOLE_RAIL, treeMode: "rail", detailsMode: "drawer", details: d };
}
