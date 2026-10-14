/**
 * Shared pieces of search: what a result looks like in a row, opening and
 * playing it, remembering it as a recent search, and the typed-part split
 * for completions. Used by the top bar's dropdown and the results page.
 */
import { backend } from "../lib/backend";
import { trackArtistLine } from "../lib/format";
import type { SearchHit, SearchKind, SearchResults } from "../lib/types";
import { player } from "../stores/player";
import { nav } from "../stores/router";
import { useSettings } from "../stores/settings";

export interface HitInfo {
  key: string;
  kind: SearchKind;
  id: string;
  title: string;
  /** The artist (or song count) line, without the type. */
  byline: string;
  /** "Artist", "Album", "Song", "Playlist". */
  typeLabel: string;
  thumb: string | null | undefined;
  round: boolean;
}

export function hitInfo(h: SearchHit): HitInfo | null {
  if (h.kind === "artist" && h.artist) {
    const a = h.artist;
    return { key: `artist:${a.id}`, kind: "artist", id: a.id, title: a.title, byline: "", typeLabel: "Artist", thumb: a.thumb, round: true };
  }
  if (h.kind === "album" && h.album) {
    const a = h.album;
    return { key: `album:${a.id}`, kind: "album", id: a.id, title: a.title, byline: a.artistTitle, typeLabel: "Album", thumb: a.thumb, round: false };
  }
  if (h.kind === "track" && h.track) {
    const t = h.track;
    return { key: `track:${t.id}`, kind: "track", id: t.id, title: t.title, byline: trackArtistLine(t), typeLabel: "Song", thumb: t.thumb, round: false };
  }
  if (h.kind === "playlist" && h.playlist) {
    const p = h.playlist;
    const songs = `${p.trackCount} ${p.trackCount === 1 ? "song" : "songs"}`;
    return { key: `playlist:${p.id}`, kind: "playlist", id: p.id, title: p.title, byline: songs, typeLabel: "Playlist", thumb: p.thumb, round: false };
  }
  return null;
}

/** "Album · Artist", "Artist", "Song · Artist": the subtitle line with the type. */
export function subtitleWithType(i: HitInfo): string {
  return i.byline ? `${i.typeLabel} · ${i.byline}` : i.typeLabel;
}

/** Remember an item opened from search (unless that's turned off). */
export function rememberHit(h: SearchHit) {
  if (useSettings.getState().settings?.searchRecents === false) return;
  const i = hitInfo(h);
  if (i) backend.searchRecentAdd(i.kind, i.id).catch(() => {});
}

/** Play a result. A song plays on its own, in a search context. */
export function playHit(h: SearchHit, songIds?: string[]) {
  const i = hitInfo(h);
  if (!i) return;
  switch (i.kind) {
    case "artist":
      return player.playArtist(i.id);
    case "album":
      return player.playAlbum(i.id);
    case "playlist":
      return player.playPlaylist(i.id);
    case "track": {
      const ids = songIds && songIds.includes(i.id) ? songIds : [i.id];
      return player.playTracks(ids, i.id, "search");
    }
  }
}

/** Open a result: its page, or for a song, play it. */
export function openHit(h: SearchHit) {
  const i = hitInfo(h);
  if (!i) return;
  if (i.kind === "track") return playHit(h);
  nav.go({ name: i.kind, id: i.id });
}

export const hitOf = {
  artist: (artist: NonNullable<SearchHit["artist"]>): SearchHit => ({ kind: "artist", artist }),
  album: (album: NonNullable<SearchHit["album"]>): SearchHit => ({ kind: "album", album }),
  track: (track: NonNullable<SearchHit["track"]>): SearchHit => ({ kind: "track", track }),
  playlist: (playlist: NonNullable<SearchHit["playlist"]>): SearchHit => ({ kind: "playlist", playlist }),
};

const sameHit = (a: SearchHit, b: SearchHit) => hitInfo(a)?.key === hitInfo(b)?.key;

/**
 * Direct results for the dropdown: the best artist first, then the top
 * result if it's something else, then albums, playlists and songs.
 */
export function dropdownHits(res: SearchResults, max = 8): SearchHit[] {
  const out: SearchHit[] = [];
  const push = (h: SearchHit) => {
    if (out.length < max && !out.some((x) => sameHit(x, h))) out.push(h);
  };
  if (res.artists[0]) push(hitOf.artist(res.artists[0]));
  if (res.top) push(res.top);
  res.albums.slice(0, 3).forEach((a) => push(hitOf.album(a)));
  res.playlists.slice(0, 2).forEach((p) => push(hitOf.playlist(p)));
  res.songs.slice(0, 4).forEach((t) => push(hitOf.track(t)));
  return out;
}

/**
 * The "All" list under the top result: the kinds taken in turns (two songs,
 * two albums, a playlist, an artist), each kind in its own rank order.
 */
export function mixedHits(res: SearchResults): SearchHit[] {
  const queues: [SearchHit[], number][] = [
    [res.songs.map(hitOf.track), 2],
    [res.albums.map(hitOf.album), 2],
    [res.playlists.map(hitOf.playlist), 1],
    [res.artists.map(hitOf.artist), 1],
  ];
  const out: SearchHit[] = [];
  const top = res.top ?? null;
  for (;;) {
    let took = false;
    for (const [q, n] of queues) {
      for (let k = 0; k < n && q.length; k++) {
        const h = q.shift()!;
        took = true;
        if (top && sameHit(h, top)) {
          k--;
          continue;
        }
        out.push(h);
      }
    }
    if (!took) return out;
  }
}

/** Lower-case, accent-free words joined by single spaces (like the core's index). */
export function fold(s: string): string {
  return s
    .normalize("NFD")
    .replace(/\p{M}/gu, "")
    .toLowerCase()
    .split(/[^\p{L}\p{N}]+/u)
    .filter(Boolean)
    .join(" ");
}

/**
 * How many leading characters of a completion the typed words cover, so
 * that part can be shown bold. 0 when the completion doesn't start with them.
 */
export function typedPrefixLength(completion: string, typed: string): number {
  const want = fold(typed);
  if (!want) return 0;
  const chars = [...completion];
  let acc = "";
  for (let i = 0; i < chars.length; i++) {
    acc += chars[i];
    const f = fold(acc);
    if (f.length >= want.length) {
      if (!f.startsWith(want)) return 0;
      // Stop at the end of the typed word, not at the next one.
      return acc.length;
    }
    if (!want.startsWith(f)) return 0;
  }
  return 0;
}
