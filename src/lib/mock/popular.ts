/**
 * The artist page's popular songs, same rule as the core: the artist's own
 * tracks plus tracks crediting them as a whole name; songs with a global
 * listener count first (highest first), then by plays and last play; one
 * copy per song (same title, length within 2 s); at most 10.
 */
import { creditKey, splitCredits, type ArtistLookup } from "../credits";
import type { Artist, Track } from "../types";

export const POPULAR_N = 10;
const SAME_SONG_MS = 2000;

export function popularTracks(tracks: readonly Track[], artist: Pick<Artist, "id" | "title">, lookup: ArtistLookup): Track[] {
  const me = creditKey(artist.title);
  const cands = tracks.filter(
    (t) =>
      t.artistId === artist.id ||
      (!!t.trackArtist &&
        t.trackArtist.toLowerCase().includes(me) &&
        splitCredits(t.trackArtist, lookup).some((c) => creditKey(c.name) === me)),
  );
  const ranked = cands
    .map((t, i) => ({ t, i }))
    .sort(
      (a, b) =>
        Number(b.t.ratingCount != null) - Number(a.t.ratingCount != null) ||
        (b.t.ratingCount ?? 0) - (a.t.ratingCount ?? 0) ||
        b.t.viewCount - a.t.viewCount ||
        (b.t.lastViewedAt ?? -1) - (a.t.lastViewedAt ?? -1) ||
        a.i - b.i,
    )
    .map(({ t }) => t);
  const out: Track[] = [];
  for (const t of ranked) {
    const dupe = out.some(
      (o) => o.id === t.id || (o.title.toLowerCase() === t.title.toLowerCase() && Math.abs(o.durationMs - t.durationMs) <= SAME_SONG_MS),
    );
    if (dupe) continue;
    out.push(t);
    if (out.length === POPULAR_N) break;
  }
  return out;
}
