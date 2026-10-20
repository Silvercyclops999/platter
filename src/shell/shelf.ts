/**
 * The record shelf view of Your Library (wide mode): albums stand as spines
 * in the current sort, wrapping onto more shelves as the window allows.
 * Everything else (artists, playlists, folders) stands on the same shelves
 * as a small grid tile. This file holds the sizes and the packing; the
 * spine itself is drawn in LibraryCollection.
 */
import type { LibEntry } from "../stores/library";

/** A spine's height, which is also the pulled-out cover's side. */
export const SPINE_HEIGHT = 208;
/** Room above the spines for the pull forward. */
export const SHELF_HEADROOM = 20;
/** The shelf board under the records. */
export const SHELF_BOARD = 14;
/** Space between one shelf and the next. */
export const SHELF_GAP = 20;
/** One shelf's height in the virtual list. */
export const SHELF_ROW = SHELF_HEADROOM + SPINE_HEIGHT + SHELF_BOARD + SHELF_GAP;

/** Spine widths: fixed, or from the song count within a narrow range. */
export const SPINE_FIXED = 34;
export const SPINE_MIN = 24;
export const SPINE_MAX = 52;
/** Space between spines. */
export const SPINE_GAP = 2;
/** Artists, playlists and folders: a tile this wide (art plus its padding). */
export const SHELF_TILE = 152;
/** Padding inside a tile. */
export const SHELF_TILE_PAD = 8;
/** The tile's art. */
export const SHELF_TILE_ART = SHELF_TILE - SHELF_TILE_PAD * 2;
/** Margin either side of a tile, so it stands apart from the spines. */
export const SHELF_TILE_MARGIN = 6;

/** An entry stands as a spine (an album you can open); anything else is a tile. */
export function isSpine(e: LibEntry): boolean {
  return e.kind === "album" && !e.folder && !e.liked && !e.bookmarks && !e.stats && !e.page && !e.unavailable;
}

/**
 * A spine's width. With the setting on, one more pixel per song from a
 * single (narrowest) up to about 30 songs (widest), so a double album reads
 * thicker. Unknown counts and the setting off get the fixed width.
 */
export function spineWidth(trackCount: number | null | undefined, byTracks: boolean): number {
  if (!byTracks || !trackCount || trackCount < 1) return SPINE_FIXED;
  return Math.max(SPINE_MIN, Math.min(SPINE_MAX, 22 + Math.round(trackCount)));
}

/** The room an entry takes along the shelf, gaps included. */
export function itemWidth(e: LibEntry, byTracks: boolean): number {
  return isSpine(e) ? spineWidth(e.trackCount, byTracks) + SPINE_GAP : SHELF_TILE + SHELF_TILE_MARGIN * 2;
}

export interface ShelfLayout {
  /** Index of the first entry on each shelf. */
  starts: number[];
  /** The shelf each entry is on. */
  shelfOf: number[];
  /** Each entry's left edge on its shelf. */
  x: number[];
  /** Each entry's width on the shelf (gap included). */
  w: number[];
}

/** Fill shelves left to right in order, starting a new shelf when the next record won't fit. */
export function packShelves(entries: LibEntry[], width: number, byTracks: boolean): ShelfLayout {
  const room = Math.max(SHELF_TILE + SHELF_TILE_MARGIN * 2, width);
  const starts: number[] = [];
  const shelfOf: number[] = [];
  const x: number[] = [];
  const w: number[] = [];
  let at = 0;
  entries.forEach((e, i) => {
    const iw = itemWidth(e, byTracks);
    if (i === 0 || at + iw > room) {
      starts.push(i);
      at = 0;
    }
    shelfOf.push(starts.length - 1);
    x.push(at);
    w.push(iw);
    at += iw;
  });
  return { starts, shelfOf, x, w };
}

/** The entry on shelf `row` whose middle is nearest `centre`. */
export function nearestOnShelf(layout: ShelfLayout, n: number, row: number, centre: number): number {
  const start = layout.starts[row];
  const end = layout.starts[row + 1] ?? n;
  let best = start;
  let bestD = Infinity;
  for (let i = start; i < end; i++) {
    const d = Math.abs(layout.x[i] + layout.w[i] / 2 - centre);
    if (d < bestD) {
      best = i;
      bestD = d;
    }
  }
  return best;
}

/** The pulled-out cover goes to the left when there's no room for it on the right. */
export function coverOnLeft(layout: ShelfLayout, i: number, width: number): boolean {
  return layout.x[i] + layout.w[i] + SPINE_HEIGHT > Math.max(width, 0) && layout.x[i] >= SPINE_HEIGHT;
}
