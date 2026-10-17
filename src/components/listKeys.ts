/**
 * Keyboard moves inside a track list, which is one Tab stop: Up / Down and
 * Home / End move between rows, Right steps into the row's links (artist,
 * album) and Left / Right walk them, and typing a few letters jumps to the
 * next song whose title starts with them.
 */

const ROW = ".track-row[data-row-key]";
const LINK = '[role="link"]';

/** Rows of the list the row is in, in order. */
export function listRows(row: HTMLElement): HTMLElement[] {
  const list = row.closest("[data-tracklist]");
  return Array.from(list?.querySelectorAll<HTMLElement>(ROW) ?? []);
}

/** The row an Up / Down / Home / End press moves to, or null. */
export function rowStep(row: HTMLElement, key: string): HTMLElement | null {
  const rows = listRows(row);
  const i = rows.indexOf(row);
  if (i < 0) return null;
  switch (key) {
    case "ArrowDown":
      return rows[i + 1] ?? null;
    case "ArrowUp":
      return rows[i - 1] ?? null;
    case "Home":
      return i > 0 ? rows[0] : null;
    case "End":
      return i < rows.length - 1 ? rows[rows.length - 1] : null;
    default:
      return null;
  }
}

/** The links inside a row (artist names, the album). */
export function rowLinks(row: HTMLElement): HTMLElement[] {
  return Array.from(row.querySelectorAll<HTMLElement>(LINK));
}

/**
 * Left / Right from the row or one of its links: the link to focus, the row
 * itself (Left from the first link), or null when the key isn't a move.
 */
export function linkStep(row: HTMLElement, from: HTMLElement, key: string): HTMLElement | null {
  if (key !== "ArrowLeft" && key !== "ArrowRight") return null;
  const links = rowLinks(row);
  if (from === row) return key === "ArrowRight" ? (links[0] ?? null) : null;
  const i = links.indexOf(from);
  if (i < 0) return null;
  if (key === "ArrowRight") return links[i + 1] ?? null;
  return i === 0 ? row : links[i - 1];
}

/** Letters typed within this long of each other make one search. */
export const TYPE_AHEAD_MS = 700;

let typed = "";
let typedAt = 0;

/** Forget what was typed (tests). */
export function resetTypeAhead() {
  typed = "";
  typedAt = 0;
}

/** A key that types a character (no modifiers other than Shift, not Space). */
export function isTypeAheadKey(e: Pick<KeyboardEvent, "key" | "ctrlKey" | "metaKey" | "altKey">): boolean {
  return e.key.length === 1 && e.key !== " " && !e.ctrlKey && !e.metaKey && !e.altKey;
}

/**
 * The row to jump to for a typed character: the next row (from `row`, wrapping)
 * whose title starts with the letters typed so far. A repeated single letter
 * cycles through the rows starting with it.
 */
export function typeAhead(row: HTMLElement, char: string, titleOf: (el: HTMLElement) => string, now = Date.now()): HTMLElement | null {
  const rows = listRows(row);
  const at = rows.indexOf(row);
  if (at < 0) return null;
  const i = typeAheadIndex(rows.map(titleOf), at, char, now);
  return i === null ? null : rows[i];
}

/**
 * The same search over a whole list's titles (a long list that only has
 * some rows on the page): the index to jump to, or null.
 */
export function typeAheadIndex(titles: readonly string[], at: number, char: string, now = Date.now()): number | null {
  typed = now - typedAt > TYPE_AHEAD_MS ? char : typed + char;
  typedAt = now;
  if (at < 0 || !titles.length) return null;
  const needle = typed.toLocaleLowerCase();
  const cycling = needle.length > 1 && [...needle].every((c) => c === needle[0]);
  const prefix = cycling ? needle[0] : needle;
  // A new search starts after the current row; a longer one may still match it.
  const start = needle.length > 1 && !cycling ? at : at + 1;
  for (let k = 0; k < titles.length; k++) {
    const i = (start + k) % titles.length;
    if (titles[i].toLocaleLowerCase().startsWith(prefix)) return i;
  }
  return null;
}
