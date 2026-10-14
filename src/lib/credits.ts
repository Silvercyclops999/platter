/**
 * Splitting a track's artist line into the artists it credits, so each name
 * can link to its artist page. Twin of `src-tauri/src/db/credits.rs` (same
 * rules): known library artist names are matched first, leftmost and longest,
 * so a name that contains "&" or "," stays whole. What is left is split on
 * ", ", " & ", " feat. ", " ft. ", " featuring ", " x " (any case) and on
 * brackets, so "A (feat. B)" works too.
 *
 * Names with no library artist link to a generated credited-artist page
 * (`credit:<name, lower case>`) when `linkAll` is on (the "Link featured
 * artists" setting). Learned names that contain separators come in with the
 * library's names (already as `credit:` ids), so they stay whole.
 */
import { trackArtistLine } from "./format";
import type { ArtistName, Track } from "./types";

export interface Credit {
  name: string;
  /** Artist page id (a library artist, or `credit:` for a credited artist); null when the name doesn't link. */
  id: string | null;
}

/** Lower-cased artist name → artist id (a Map works). */
export interface ArtistLookup {
  has(key: string): boolean;
  get(key: string): string | undefined;
}

export const creditKey = (name: string): string => name.trim().toLowerCase();

/** Id prefix of a credited artist's page. */
export const CREDIT_PREFIX = "credit:";

/** The page id for a credited name with no library artist. */
export const creditId = (name: string): string => `${CREDIT_PREFIX}${creditKey(name)}`;

/** Whether an artist id is a credited artist (no Plex entry). */
export const isCreditId = (id: string): boolean => id.startsWith(CREDIT_PREFIX);

export function buildArtistLookup(names: readonly ArtistName[]): ArtistLookup {
  const m = new Map<string, string>();
  for (const a of names) {
    const k = creditKey(a.title);
    if (k && !m.has(k)) m.set(k, a.id);
  }
  return m;
}

const SEP_WORDS = ["featuring", "feat.", "feat", "ft.", "ft", "x"];
const isWs = (c: string | undefined) => c !== undefined && /\s/.test(c);

/** [start, end) ranges of the separators in `line`. */
function separatorSpans(line: string): [number, number][] {
  const spans: [number, number][] = [];
  const n = line.length;
  let i = 0;
  while (i < n) {
    const c = line[i];
    const prev = i > 0 ? line[i - 1] : undefined;
    const next = line[i + 1];
    if (c === "," || c === "(" || c === ")" || c === "[" || c === "]") {
      spans.push([i, i + 1]);
      i += 1;
      continue;
    }
    if (c === "&" && isWs(prev) && isWs(next)) {
      spans.push([i, i + 1]);
      i += 1;
      continue;
    }
    if (isWs(prev) || prev === "(" || prev === "[") {
      const word = SEP_WORDS.find((w) => {
        const end = i + w.length;
        return end < n && line.slice(i, end).toLowerCase() === w && isWs(line[end]);
      });
      // A lone "x" only counts between spaces, never after a bracket.
      if (word && (word !== "x" || isWs(prev))) {
        spans.push([i, i + word.length]);
        i += word.length;
        continue;
      }
    }
    i += 1;
  }
  return spans;
}

/** Non-empty pieces between separators, as trimmed [start, end) ranges. */
function parts(line: string): [number, number][] {
  const out: [number, number][] = [];
  const push = (a: number, b: number) => {
    const piece = line.slice(a, b);
    const lead = piece.length - piece.trimStart().length;
    const trimmed = piece.trim();
    if (trimmed) out.push([a + lead, a + lead + trimmed.length]);
  };
  let start = 0;
  for (const [a, b] of separatorSpans(line)) {
    push(start, a);
    start = b;
  }
  push(start, line.length);
  return out;
}

/** Credited names in line order, each once (case-insensitive), with ids where known. */
export function splitCredits(line: string, lookup: ArtistLookup, linkAll = false): Credit[] {
  const ps = parts(line);
  const out: Credit[] = [];
  const seen = new Set<string>();
  let i = 0;
  while (i < ps.length) {
    // Longest run of pieces starting here that is a known name.
    let end = i;
    let name = line.slice(ps[i][0], ps[i][1]).trim();
    for (let j = ps.length - 1; j > i; j--) {
      const cand = line.slice(ps[i][0], ps[j][1]).trim();
      if (lookup.has(creditKey(cand))) {
        end = j;
        name = cand;
        break;
      }
    }
    const key = creditKey(name);
    if (!seen.has(key)) {
      seen.add(key);
      out.push({ name, id: lookup.get(key) ?? (linkAll && key ? creditId(name) : null) });
    }
    i = end + 1;
  }
  return out;
}

/**
 * Credits for a track's artist line (the track artists, or the album artist
 * when there are none). The album artist always links, even before the
 * library's names have loaded.
 */
export function trackCredits(
  t: Pick<Track, "trackArtist" | "artistTitle" | "artistId">,
  lookup: ArtistLookup,
  linkAll = false,
): Credit[] {
  const albumKey = creditKey(t.artistTitle);
  const withAlbum: ArtistLookup =
    albumKey && t.artistId
      ? {
          has: (k) => k === albumKey || lookup.has(k),
          get: (k) => (k === albumKey ? t.artistId : lookup.get(k)),
        }
      : lookup;
  return splitCredits(trackArtistLine(t), withAlbum, linkAll);
}
