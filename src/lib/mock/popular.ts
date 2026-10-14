/**
 * The artist page's popular songs, same rule as the core: candidates are the
 * artist's own tracks plus tracks crediting them as a whole name. Songs on
 * the global top list (made-up listen counts here) come first in list order,
 * matched by normalized title; the rest follow by plays, then last play,
 * with the rating count only breaking ties. One copy per song; at most 10.
 * "myPlays" ignores the global list. Also the credited-artist helpers.
 */
import { creditKey, splitCredits, type ArtistLookup } from "../credits";
import type { PopularSource, Track } from "../types";
import { hashString } from "./fixtures";

export const POPULAR_N = 10;
const SAME_SONG_MS = 2000;
const VERSION_WORDS = ["remaster", "remastered", "version", "mono", "stereo", "edit", "mix", "live", "demo", "single", "mixed"];

/** Twin of the core's title folding: brackets, feat. and version suffixes dropped, punctuation collapsed. */
export function normalizeTitle(s: string): string {
  let out = "";
  let depth = 0;
  for (const ch of s.normalize("NFKC").toLowerCase()) {
    if ("([{".includes(ch)) depth++;
    else if (")]}".includes(ch)) depth = Math.max(0, depth - 1);
    else if (depth === 0) out += ch;
  }
  const dash = out.indexOf(" - ");
  if (dash >= 0) {
    const tail = out.slice(dash + 3).split(/[^\p{L}\p{N}]+/u);
    if (tail.some((t) => VERSION_WORDS.includes(t) || t.startsWith("remaster"))) out = out.slice(0, dash);
  }
  for (const sep of [" feat. ", " ft. ", " featuring "]) {
    const i = out.indexOf(sep);
    if (i >= 0) out = out.slice(0, i);
  }
  return out
    .replace(/['’`]/g, "")
    .replace(/&/g, " and ")
    .replace(/[^\p{L}\p{N}]+/gu, " ")
    .trim();
}

/** Made-up global top list for an artist: released songs with listen counts, most first. */
export function globalTop(tracks: readonly Track[]): string[] {
  const best = new Map<string, number>();
  for (const t of tracks) {
    if (t.ratingCount == null) continue; // unreleased and rare songs aren't on it
    const key = normalizeTitle(t.title);
    const listens = hashString(`listens:${key}`) % 1_000_000;
    best.set(key, (best.get(key) ?? 0) + listens);
  }
  return [...best.entries()].sort((a, b) => b[1] - a[1]).slice(0, 100).map(([k]) => k);
}

/** Whether a track's artist line credits `name` as a whole name. */
export function credits(t: Track, name: string, lookup: ArtistLookup): boolean {
  const me = creditKey(name);
  return !!t.trackArtist && t.trackArtist.toLowerCase().includes(me) && splitCredits(t.trackArtist, lookup).some((c) => creditKey(c.name) === me);
}

export function rankPopular(cands: readonly Track[], global: string[] | null): Track[] {
  const rank = new Map((global ?? []).map((k, i) => [k, i] as const));
  const keyed = cands.map((t, i) => ({ t, i, k: normalizeTitle(t.title), r: rank.get(normalizeTitle(t.title)) }));
  keyed.sort((a, b) => {
    if (a.r != null && b.r != null) return a.r - b.r || b.t.viewCount - a.t.viewCount || a.i - b.i;
    if (a.r != null) return -1;
    if (b.r != null) return 1;
    return (
      b.t.viewCount - a.t.viewCount ||
      (b.t.lastViewedAt ?? -1) - (a.t.lastViewedAt ?? -1) ||
      (b.t.ratingCount ?? -1) - (a.t.ratingCount ?? -1) ||
      a.i - b.i
    );
  });
  const out: typeof keyed = [];
  for (const c of keyed) {
    const dupe = out.some(
      (o) => o.t.id === c.t.id || (o.r != null && o.r === c.r) || (o.k === c.k && Math.abs(o.t.durationMs - c.t.durationMs) <= SAME_SONG_MS),
    );
    if (dupe) continue;
    out.push(c);
    if (out.length === POPULAR_N) break;
  }
  return out.map((c) => c.t);
}

export function popularTracks(
  tracks: readonly Track[],
  artist: { id: string; title: string },
  lookup: ArtistLookup,
  source: PopularSource = "global",
): Track[] {
  const cands = tracks.filter((t) => t.artistId === artist.id || credits(t, artist.title, lookup));
  return rankPopular(cands, source === "global" ? globalTop(cands) : null);
}
