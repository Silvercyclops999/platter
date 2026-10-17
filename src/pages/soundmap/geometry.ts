/**
 * The sound map's maths, kept apart from drawing so it can be tested:
 * the points as columns, the view (where the map sits on screen), finding
 * the point under the pointer quickly, the lasso, and the colour groups.
 */
import type { SoundMapChunk } from "../../lib/types";

/** Every point, in columns. */
export interface MapData {
  ids: string[];
  x: Float32Array;
  y: Float32Array;
  /** Genre name per point (null: none). */
  genre: (string | null)[];
  /** 0: unknown. */
  year: Int32Array;
  unreleased: Uint8Array;
  /** Index by song id. */
  index: Map<string, number>;
}

export function emptyData(): MapData {
  return { ids: [], x: new Float32Array(), y: new Float32Array(), genre: [], year: new Int32Array(), unreleased: new Uint8Array(), index: new Map() };
}

/** Joins the chunks the core sent into one set of columns. */
export function joinChunks(chunks: SoundMapChunk[]): MapData {
  const n = chunks.reduce((a, c) => a + c.ids.length, 0);
  const d: MapData = {
    ids: new Array(n),
    x: new Float32Array(n),
    y: new Float32Array(n),
    genre: new Array(n),
    year: new Int32Array(n),
    unreleased: new Uint8Array(n),
    index: new Map(),
  };
  let i = 0;
  for (const c of chunks) {
    for (let j = 0; j < c.ids.length; j++, i++) {
      d.ids[i] = c.ids[j];
      d.x[i] = c.x[j];
      d.y[i] = c.y[j];
      const g = c.genre[j];
      d.genre[i] = g >= 0 ? (c.genres[g] ?? null) : null;
      d.year[i] = c.year[j];
      d.unreleased[i] = c.unreleased[j] ? 1 : 0;
      d.index.set(c.ids[j], i);
    }
  }
  return d;
}

/** Where the map sits: the map point at the middle of the screen, and pixels per map unit. */
export interface View {
  cx: number;
  cy: number;
  scale: number;
}

export const MIN_ZOOM = 0.5;
export const MAX_ZOOM = 400;

/** A view that shows every point with a margin. */
export function fitView(d: MapData, width: number, height: number): View {
  if (d.ids.length === 0) return { cx: 0, cy: 0, scale: Math.min(width, height) / 2.4 };
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (let i = 0; i < d.ids.length; i++) {
    minX = Math.min(minX, d.x[i]);
    maxX = Math.max(maxX, d.x[i]);
    minY = Math.min(minY, d.y[i]);
    maxY = Math.max(maxY, d.y[i]);
  }
  const w = Math.max(maxX - minX, 0.05);
  const h = Math.max(maxY - minY, 0.05);
  const scale = Math.min(width / (w * 1.15), height / (h * 1.15));
  return { cx: (minX + maxX) / 2, cy: (minY + maxY) / 2, scale };
}

/** Map point to screen (y grows downwards on screen, upwards on the map). */
export function toScreen(v: View, w: number, h: number, x: number, y: number): [number, number] {
  return [w / 2 + (x - v.cx) * v.scale, h / 2 - (y - v.cy) * v.scale];
}

export function toMap(v: View, w: number, h: number, sx: number, sy: number): [number, number] {
  return [v.cx + (sx - w / 2) / v.scale, v.cy - (sy - h / 2) / v.scale];
}

/** Zoom by `factor`, keeping the map point under (sx, sy) where it is. */
export function zoomAt(v: View, w: number, h: number, sx: number, sy: number, factor: number, base: number): View {
  const scale = Math.min(Math.max(v.scale * factor, base * MIN_ZOOM), base * MAX_ZOOM);
  const [mx, my] = toMap(v, w, h, sx, sy);
  return { scale, cx: mx - (sx - w / 2) / scale, cy: my + (sy - h / 2) / scale };
}

/** The same zoom (relative to the fitted view) after the fitted view's scale moved from `from` to `to`. */
export function rebase(v: View, from: number, to: number): View {
  if (!(from > 0) || !(to > 0) || from === to) return v;
  return { ...v, scale: v.scale * (to / from) };
}

/** Wheel distance in pixels, whatever unit the wheel reported (lines or pages too). */
export function wheelPixels(delta: number, mode: number, page: number): number {
  if (mode === 1) return delta * 16;
  if (mode === 2) return delta * page;
  return delta;
}

/**
 * The nearest point from `from` in a direction (dx, dy in map units, y up):
 * only points within 45 degrees of it count, and points off to the side
 * count as further away. -1 when there's none.
 */
export function nearestToward(d: MapData, from: number, dx: number, dy: number): number {
  const n = d.ids.length;
  if (from < 0 || from >= n) return -1;
  const x0 = d.x[from];
  const y0 = d.y[from];
  let best = -1;
  let bestD = Infinity;
  for (let i = 0; i < n; i++) {
    if (i === from) continue;
    const ex = d.x[i] - x0;
    const ey = d.y[i] - y0;
    const along = ex * dx + ey * dy;
    if (along <= 0) continue;
    const side = Math.abs(ex * dy - ey * dx);
    if (side > along) continue;
    const score = along * along + 4 * side * side;
    if (score < bestD) {
      bestD = score;
      best = i;
    }
  }
  return best;
}

/** Dot radius in pixels: a little bigger as you zoom in. */
export function dotRadius(v: View, base: number): number {
  return Math.min(6, Math.max(1.6, 2.2 * Math.sqrt(v.scale / Math.max(base, 1e-6))));
}

/** A coarse grid over the map for finding the point under the pointer. */
export interface Grid {
  minX: number;
  minY: number;
  cell: number;
  cols: number;
  rows: number;
  cells: Map<number, number[]>;
}

export function buildGrid(d: MapData): Grid {
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (let i = 0; i < d.ids.length; i++) {
    minX = Math.min(minX, d.x[i]);
    maxX = Math.max(maxX, d.x[i]);
    minY = Math.min(minY, d.y[i]);
    maxY = Math.max(maxY, d.y[i]);
  }
  if (!isFinite(minX)) return { minX: 0, minY: 0, cell: 1, cols: 1, rows: 1, cells: new Map() };
  const side = Math.max(1, Math.ceil(Math.sqrt(d.ids.length / 4)));
  const cell = Math.max(maxX - minX, maxY - minY, 1e-3) / side;
  const cols = Math.floor((maxX - minX) / cell) + 1;
  const rows = Math.floor((maxY - minY) / cell) + 1;
  const cells = new Map<number, number[]>();
  for (let i = 0; i < d.ids.length; i++) {
    const k = Math.floor((d.y[i] - minY) / cell) * cols + Math.floor((d.x[i] - minX) / cell);
    const list = cells.get(k);
    if (list) list.push(i);
    else cells.set(k, [i]);
  }
  return { minX, minY, cell, cols, rows, cells };
}

/** The point nearest (mx, my) within `radius` map units, or -1. */
export function nearest(d: MapData, g: Grid, mx: number, my: number, radius: number): number {
  const c0 = Math.floor((mx - radius - g.minX) / g.cell);
  const c1 = Math.floor((mx + radius - g.minX) / g.cell);
  const r0 = Math.floor((my - radius - g.minY) / g.cell);
  const r1 = Math.floor((my + radius - g.minY) / g.cell);
  let best = -1;
  let bestD = radius * radius;
  for (let r = Math.max(0, r0); r <= Math.min(g.rows - 1, r1); r++) {
    for (let c = Math.max(0, c0); c <= Math.min(g.cols - 1, c1); c++) {
      for (const i of g.cells.get(r * g.cols + c) ?? []) {
        const dx = d.x[i] - mx;
        const dy = d.y[i] - my;
        const dd = dx * dx + dy * dy;
        if (dd <= bestD) {
          bestD = dd;
          best = i;
        }
      }
    }
  }
  return best;
}

/** Whether (x, y) is inside the closed shape `poly` (even-odd rule). */
export function inPolygon(poly: [number, number][], x: number, y: number): boolean {
  let inside = false;
  for (let i = 0, j = poly.length - 1; i < poly.length; j = i++) {
    const [xi, yi] = poly[i];
    const [xj, yj] = poly[j];
    if (yi > y !== yj > y && x < ((xj - xi) * (y - yi)) / (yj - yi) + xi) inside = !inside;
  }
  return inside;
}

/** The points inside a lasso drawn in map units. */
export function lassoSelect(d: MapData, poly: [number, number][]): number[] {
  if (poly.length < 3) return [];
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const [x, y] of poly) {
    minX = Math.min(minX, x);
    maxX = Math.max(maxX, x);
    minY = Math.min(minY, y);
    maxY = Math.max(maxY, y);
  }
  const out: number[] = [];
  for (let i = 0; i < d.ids.length; i++) {
    const x = d.x[i];
    const y = d.y[i];
    if (x < minX || x > maxX || y < minY || y > maxY) continue;
    if (inPolygon(poly, x, y)) out.push(i);
  }
  return out;
}

// ------------------------------------------------------------ colour groups

export type ColourBy = "genre" | "era";

/** Genres shown in their own colour; the rest share "Other". */
export const GENRE_COLOURS = 9;

export interface Legend {
  /** Group per point (index into `groups`). */
  group: Uint8Array;
  groups: { label: string; count: number; /** Palette slot; -1: the neutral colour. */ slot: number }[];
}

export const ERA_GROUPS = ["Before 1970", "1970s", "1980s", "1990s", "2000s", "2010s", "2020s"] as const;

export function eraOf(year: number): number {
  if (!year) return -1;
  if (year < 1970) return 0;
  return Math.min(ERA_GROUPS.length - 1, Math.floor((year - 1960) / 10));
}

export function legend(d: MapData, by: ColourBy): Legend {
  const n = d.ids.length;
  const group = new Uint8Array(n);
  if (by === "era") {
    const counts = new Array(ERA_GROUPS.length + 1).fill(0);
    for (let i = 0; i < n; i++) {
      const e = eraOf(d.year[i]);
      const g = e < 0 ? ERA_GROUPS.length : e;
      group[i] = g;
      counts[g]++;
    }
    const groups = [...ERA_GROUPS.map((label, i) => ({ label, count: counts[i], slot: i })), { label: "Year unknown", count: counts[ERA_GROUPS.length], slot: -1 }];
    return { group, groups };
  }
  const counts = new Map<string, number>();
  for (const g of d.genre) if (g) counts.set(g, (counts.get(g) ?? 0) + 1);
  const top = [...counts.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).slice(0, GENRE_COLOURS);
  const slot = new Map(top.map(([name], i) => [name, i]));
  let other = 0;
  for (let i = 0; i < n; i++) {
    const s = d.genre[i] ? slot.get(d.genre[i]!) : undefined;
    if (s === undefined) {
      group[i] = top.length;
      other++;
    } else group[i] = s;
  }
  const groups = top.map(([label, count], i) => ({ label, count, slot: i }));
  groups.push({ label: top.length ? "Other" : "No genre", count: other, slot: -1 });
  return { group, groups };
}
