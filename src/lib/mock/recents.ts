/**
 * The Recents page for the mock backend, following the core's rules
 * (db/history.rs): plays group into listening sessions by context with
 * gaps under 30 minutes; likes show as "songs added to Liked Songs".
 */
import type { HomeItem, RecentsEntry, Track } from "../types";
import type { Fixtures } from "./fixtures";

export const SESSION_GAP_MS = 30 * 60 * 1000;
export const RECENTS_MAX = 300;
const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;

/** One play in the mock listen log (unix ms). */
export interface MockListen {
  trackId: string;
  at: number;
  end: number;
  kind: "album" | "artist" | "playlist" | "liked" | "songs";
  id?: string;
}

function runs<T>(events: T[], key: (e: T) => string, span: (e: T) => [number, number]): T[][] {
  const out: T[][] = [];
  let lastEnd = -Infinity;
  for (const e of events) {
    const [start, end] = span(e);
    const run = out[out.length - 1];
    const joins = !!run && key(run[run.length - 1]) === key(e) && start - lastEnd < SESSION_GAP_MS;
    if (joins) run.push(e);
    else out.push([e]);
    lastEnd = joins ? Math.max(lastEnd, end) : Math.max(start, end);
  }
  return out;
}

function itemFor(fx: Fixtures, kind: MockListen["kind"], id?: string): HomeItem | null {
  if (!id) return null;
  if (kind === "album") {
    const album = fx.albums.find((a) => a.id === id);
    return album ? { kind, album } : null;
  }
  if (kind === "artist") {
    const artist = fx.artists.find((a) => a.id === id);
    return artist ? { kind, artist } : null;
  }
  if (kind === "playlist") {
    const playlist = fx.playlists.find((p) => p.id === id);
    return playlist ? { kind, playlist } : null;
  }
  return null;
}

export function buildRecents(
  fx: Fixtures,
  log: MockListen[],
  liked: Track[],
  trackById: (id: string) => Track | undefined,
  now = Date.now(),
  limit = RECENTS_MAX,
): RecentsEntry[] {
  const since = now - 90 * DAY;
  const plays = log
    .filter((p) => p.at >= since && trackById(p.trackId))
    .sort((a, b) => a.at - b.at)
    .map((p) => {
      const item = p.kind === "album" || p.kind === "artist" || p.kind === "playlist" ? itemFor(fx, p.kind, p.id) : null;
      const kind: RecentsEntry["kind"] = item ? "context" : p.kind === "liked" ? "liked" : "songs";
      return { p, item, kind, key: item ? `${p.kind}:${p.id}` : kind };
    });
  const out: RecentsEntry[] = runs(plays, (x) => x.key, (x) => [x.p.at, x.p.end]).map((run) => ({
    kind: run[0].kind,
    item: run[0].item,
    at: Math.max(...run.map((x) => x.p.at)),
    tracks: run.map((x) => trackById(x.p.trackId)!).reverse(),
  }));
  const likes = liked
    .filter((t) => t.ratedAt && t.ratedAt * 1000 >= since)
    .sort((a, b) => a.ratedAt! - b.ratedAt!);
  for (const run of runs(likes, () => "liked", (t) => [t.ratedAt! * 1000, t.ratedAt! * 1000])) {
    out.push({ kind: "likedAdded", item: null, at: Math.max(...run.map((t) => t.ratedAt! * 1000)), tracks: [...run].reverse() });
  }
  return out.sort((a, b) => b.at - a.at).slice(0, limit);
}

/**
 * Invented listening: albums, a playlist, an artist, Liked Songs and loose
 * songs over the last week, so the page has Today, Yesterday and dates.
 */
export function seedListens(fx: Fixtures, likedIds: string[], now = Date.now()): MockListen[] {
  const out: MockListen[] = [];
  const tracksOf = (albumId: string) => fx.tracks.filter((t) => t.albumId === albumId);
  const add = (start: number, kind: MockListen["kind"], ids: string[], id?: string) => {
    let at = start;
    for (const trackId of ids) {
      const t = fx.tracks.find((x) => x.id === trackId);
      const len = Math.min(t?.durationMs ?? 3 * MIN, 6 * MIN);
      out.push({ trackId, at, end: at + len, kind, id });
      at += len;
    }
  };
  const albums = fx.albums.filter((a) => tracksOf(a.id).length >= 4);
  const songs = (albumIndex: number, n: number) => tracksOf(albums[albumIndex % albums.length]?.id ?? "").slice(0, n).map((t) => t.id);
  const pl = fx.playlists[0];
  const plTracks = pl ? (fx.playlistTracks.get(pl.id) ?? []).slice(0, 4) : [];
  const artist = albums[3] ? fx.artists.find((a) => a.id === albums[3].artistId) : undefined;
  if (!albums.length) return out;
  // Today.
  add(now - 25 * MIN, "album", songs(0, 3), albums[0].id);
  add(now - 70 * MIN, "liked", likedIds.slice(0, 4));
  if (pl) add(now - 3 * HOUR, "playlist", plTracks, pl.id);
  add(now - 5 * HOUR, "album", songs(0, 5), albums[0].id);
  // Yesterday.
  add(now - DAY - 2 * HOUR, "songs", [songs(5, 1)[0], songs(6, 1)[0]].filter(Boolean));
  if (artist) add(now - DAY - 4 * HOUR, "artist", songs(3, 3), artist.id);
  add(now - DAY - 7 * HOUR, "album", songs(1, 8), albums[1].id);
  // Earlier this week.
  add(now - 3 * DAY, "album", songs(2, 6), albums[2].id);
  add(now - 3 * DAY - 3 * HOUR, "liked", likedIds.slice(4, 6));
  add(now - 5 * DAY, "album", songs(4, 10), albums[4 % albums.length].id);
  return out;
}
