/** Liked Songs page logic: sort, find, filter chips and the saved view. */
import { trackArtistLine } from "../lib/format";
import type { Track } from "../lib/types";
import { fold } from "../stores/library";

export type LikedSort = "added" | "title" | "artist" | "album";
export type LikedView = "list" | "compact";

export const LIKED_SORT_LABEL: Record<LikedSort, string> = {
  title: "Title",
  added: "Date added",
  artist: "Artist",
  album: "Album",
};
export const LIKED_VIEW_LABEL: Record<LikedView, string> = { compact: "Compact", list: "List" };

/** Chips shown at most. */
export const MAX_CHIPS = 12;

export interface Chip {
  tag: string;
  count: number;
}

/** The genre and mood tags of the liked songs, most common first (ties by name), top `max`. */
export function likedChips(tracks: Pick<Track, "genres">[], max = MAX_CHIPS): Chip[] {
  const counts = new Map<string, Chip>();
  for (const t of tracks) {
    const seen = new Set<string>();
    for (const g of t.genres ?? []) {
      const key = g.trim().toLowerCase();
      if (!key || seen.has(key)) continue;
      seen.add(key);
      const c = counts.get(key);
      if (c) c.count++;
      else counts.set(key, { tag: g.trim(), count: 1 });
    }
  }
  return [...counts.values()].sort((a, b) => b.count - a.count || a.tag.localeCompare(b.tag)).slice(0, max);
}

const hasTag = (t: Pick<Track, "genres">, tag: string) => (t.genres ?? []).some((g) => g.trim().toLowerCase() === tag.toLowerCase());

/** Find in the list: every word appears in the title, artists or album (case and accents ignored). */
export function matchesFind(t: Track, query: string): boolean {
  const words = fold(query).split(/\s+/).filter(Boolean);
  if (!words.length) return true;
  const hay = fold(`${t.title} ${trackArtistLine(t)} ${t.artistTitle} ${t.albumTitle}`);
  return words.every((w) => hay.includes(w));
}

const cmp = (a: string, b: string) => a.localeCompare(b, undefined, { sensitivity: "base", numeric: true });

/** Liked songs in a sort: date added is newest first; the rest A to Z. */
export function sortLiked(tracks: Track[], sort: LikedSort): Track[] {
  const by: Record<LikedSort, (a: Track, b: Track) => number> = {
    added: (a, b) => (b.ratedAt ?? 0) - (a.ratedAt ?? 0),
    title: (a, b) => cmp(a.title, b.title),
    artist: (a, b) => cmp(trackArtistLine(a), trackArtistLine(b)) || cmp(a.albumTitle, b.albumTitle) || a.discNumber - b.discNumber || a.trackNumber - b.trackNumber,
    album: (a, b) => cmp(a.albumTitle, b.albumTitle) || a.discNumber - b.discNumber || a.trackNumber - b.trackNumber,
  };
  return [...tracks].sort(by[sort]);
}

/** What the table shows: the chip, then find, then the sort. */
export function likedRows(tracks: Track[], opts: { sort: LikedSort; chip: string | null; query: string }): Track[] {
  const kept = tracks.filter((t) => (!opts.chip || hasTag(t, opts.chip)) && matchesFind(t, opts.query));
  return sortLiked(kept, opts.sort);
}

// ---------------------------------------------------------------- saved view (per device)

const KEY = "platter.liked-view.v1"; // gitleaks:allow (localStorage key name, not a secret)

export interface LikedPrefs {
  sort: LikedSort;
  view: LikedView;
}

export const LIKED_PREFS_DEFAULT: LikedPrefs = { sort: "added", view: "list" };

export function loadLikedPrefs(): LikedPrefs {
  try {
    const p = JSON.parse(localStorage.getItem(KEY) ?? "{}") as Partial<LikedPrefs>;
    return {
      sort: p.sort && p.sort in LIKED_SORT_LABEL ? p.sort : LIKED_PREFS_DEFAULT.sort,
      view: p.view === "compact" || p.view === "list" ? p.view : LIKED_PREFS_DEFAULT.view,
    };
  } catch {
    return { ...LIKED_PREFS_DEFAULT };
  }
}

export function saveLikedPrefs(p: LikedPrefs) {
  try {
    localStorage.setItem(KEY, JSON.stringify(p));
  } catch {
    /* storage unavailable: this session only */
  }
}
