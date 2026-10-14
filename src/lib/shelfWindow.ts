/**
 * Which cards of a horizontal shelf get their artwork.
 *
 * Decoded art is most of the WebView's memory, so a shelf only gives an image
 * to the cards on screen plus a margin either side. Cards that scroll further
 * away than a wider "keep" margin drop their image again (their box stays, so
 * nothing shifts). The gap between the two margins stops images flickering in
 * and out when the row moves back and forth near an edge.
 */

/** Half-open index range [start, end). */
export interface ArtRange {
  start: number;
  end: number;
}

export interface ShelfMetrics {
  /** Current horizontal scroll offset of the row (px). */
  scrollLeft: number;
  /** Visible width of the row (px). */
  viewport: number;
  /** Distance from one card's left edge to the next (px). */
  itemWidth: number;
  count: number;
}

/** Load art this many viewports ahead of the visible cards, each side. */
export const SHELF_LOAD_MARGIN = 0.5;
/** Release art once a card is further than this many viewports away. */
export const SHELF_KEEP_MARGIN = 1.5;

/** Cards that overlap the visible area grown by `marginPx` on each side. */
export function rangeWithin(m: ShelfMetrics, marginPx: number): ArtRange {
  if (m.count <= 0) return { start: 0, end: 0 };
  if (!(m.itemWidth > 0) || !(m.viewport > 0)) return { start: 0, end: m.count };
  const left = m.scrollLeft - marginPx;
  const right = m.scrollLeft + m.viewport + marginPx;
  const start = Math.max(0, Math.floor(left / m.itemWidth));
  const end = Math.min(m.count, Math.ceil(right / m.itemWidth));
  return { start: Math.min(start, end), end };
}

/**
 * Next art range: everything near the view, plus whatever already had art and
 * is still within the keep margin.
 */
export function nextArtRange(
  prev: ArtRange | null,
  m: ShelfMetrics,
  load = SHELF_LOAD_MARGIN,
  keep = SHELF_KEEP_MARGIN,
): ArtRange {
  const near = rangeWithin(m, m.viewport * load);
  if (!prev || prev.end <= prev.start) return near;
  const far = rangeWithin(m, m.viewport * keep);
  // Keep the previous cards only where they overlap the far range, and only
  // if they touch the near range (so the result stays one contiguous run).
  const keptStart = Math.max(prev.start, far.start);
  const keptEnd = Math.min(prev.end, far.end);
  if (keptEnd <= keptStart || keptEnd < near.start || keptStart > near.end) return near;
  return { start: Math.min(near.start, keptStart), end: Math.max(near.end, keptEnd) };
}

export function sameRange(a: ArtRange | null, b: ArtRange | null): boolean {
  return a === b || (!!a && !!b && a.start === b.start && a.end === b.end);
}
