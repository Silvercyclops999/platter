/**
 * Home rows for the mock backend, following the core's rules (db/queries.rs):
 * recently played mixes albums, artists and playlists; the shortcut grid is
 * up to 6 recent items, then the most played (album, artist, playlist in
 * turn), then more recent ones, 8 at most and no repeats.
 */
import type { Album, Artist, HomeData, HomeItem, HomeShelfId, Playlist } from "../types";
import type { Fixtures } from "./fixtures";

export const HOME_ROW = 12;
export const HOME_SHORTCUTS = 8;
const SHORTCUT_RECENT = 6;
export const SHELF_ALL_MAX = 200;

/** A play started from an album, artist or playlist (unix seconds). */
export interface ContextPlay {
  kind: "album" | "artist" | "playlist";
  id: string;
  at: number;
}

export const homeKey = (i: HomeItem): string =>
  `${i.kind}:${i.kind === "album" ? i.album.id : i.kind === "artist" ? i.artist.id : i.playlist.id}`;

export function shortcuts(recent: HomeItem[], most: HomeItem[]): HomeItem[] {
  const seen = new Set<string>();
  const out: HomeItem[] = [];
  for (const item of [...recent.slice(0, SHORTCUT_RECENT), ...most, ...recent.slice(SHORTCUT_RECENT)]) {
    if (out.length >= HOME_SHORTCUTS) break;
    const k = homeKey(item);
    if (seen.has(k)) continue;
    seen.add(k);
    out.push(item);
  }
  return out;
}

export class MockHome {
  constructor(
    private fx: Fixtures,
    private plays: ContextPlay[],
  ) {}

  private find(kind: ContextPlay["kind"], id: string): HomeItem | null {
    if (kind === "album") {
      const album = this.fx.albums.find((a) => a.id === id);
      return album ? { kind, album } : null;
    }
    if (kind === "artist") {
      const artist = this.fx.artists.find((a) => a.id === id);
      return artist ? { kind, artist } : null;
    }
    const playlist = this.fx.playlists.find((p) => p.id === id);
    return playlist ? { kind, playlist } : null;
  }

  recentlyPlayed(limit: number): HomeItem[] {
    const at = new Map<string, ContextPlay>();
    const note = (p: ContextPlay) => {
      const k = `${p.kind}:${p.id}`;
      if ((at.get(k)?.at ?? -Infinity) < p.at) at.set(k, p);
    };
    for (const a of this.fx.albums) if (a.lastViewedAt) note({ kind: "album", id: a.id, at: a.lastViewedAt });
    this.plays.forEach(note);
    return [...at.values()]
      .sort((a, b) => b.at - a.at)
      .map((p) => this.find(p.kind, p.id))
      .filter((i): i is HomeItem => !!i)
      .slice(0, limit);
  }

  topArtists(limit: number): Artist[] {
    return this.fx.artists
      .filter((a) => a.viewCount > 0)
      .sort((a, b) => b.viewCount - a.viewCount || a.sortTitle.localeCompare(b.sortTitle))
      .slice(0, limit);
  }

  playlists(limit: number): Playlist[] {
    const last = new Map<string, number>();
    for (const p of this.plays) if (p.kind === "playlist") last.set(p.id, Math.max(last.get(p.id) ?? 0, p.at));
    return [...this.fx.playlists]
      .sort((a, b) => (last.get(b.id) ?? -1) - (last.get(a.id) ?? -1) || a.title.localeCompare(b.title))
      .slice(0, limit);
  }

  recentlyAdded(limit: number): Album[] {
    return [...this.fx.albums].sort((a, b) => b.addedAt - a.addedAt).slice(0, limit);
  }

  jumpBackIn(limit: number): Album[] {
    const recent = new Set(
      this.recentlyPlayed(6)
        .filter((i) => i.kind === "album")
        .map((i) => (i.kind === "album" ? i.album.id : "")),
    );
    return [...this.fx.albums]
      .filter((a) => a.viewCount > 0 && !recent.has(a.id))
      .sort((a, b) => b.viewCount - a.viewCount)
      .slice(0, limit);
  }

  private mostPlayed(perKind: number): HomeItem[] {
    const albums: HomeItem[] = [...this.fx.albums]
      .filter((a) => a.viewCount > 0)
      .sort((a, b) => b.viewCount - a.viewCount)
      .slice(0, perKind)
      .map((album) => ({ kind: "album", album }));
    const artists: HomeItem[] = this.topArtists(perKind).map((artist) => ({ kind: "artist", artist }));
    const counts = new Map<string, number>();
    for (const p of this.plays) if (p.kind === "playlist") counts.set(p.id, (counts.get(p.id) ?? 0) + 1);
    const playlists: HomeItem[] = this.fx.playlists
      .filter((p) => counts.has(p.id))
      .sort((a, b) => counts.get(b.id)! - counts.get(a.id)!)
      .slice(0, perKind)
      .map((playlist) => ({ kind: "playlist", playlist }));
    const out: HomeItem[] = [];
    for (let i = 0; i < perKind; i++) for (const l of [albums, artists, playlists]) if (l[i]) out.push(l[i]);
    return out;
  }

  home(): HomeData {
    const recent = this.recentlyPlayed(Math.max(HOME_ROW, HOME_SHORTCUTS));
    return {
      shortcuts: shortcuts(recent, this.mostPlayed(HOME_SHORTCUTS)),
      recentlyPlayed: recent.slice(0, HOME_ROW),
      jumpBackIn: this.jumpBackIn(HOME_ROW),
      playlists: this.playlists(HOME_ROW),
      recentlyAdded: this.recentlyAdded(HOME_ROW),
      topArtists: this.topArtists(HOME_ROW),
    };
  }

  shelf(shelf: HomeShelfId, limit = SHELF_ALL_MAX): HomeItem[] {
    const n = Math.min(SHELF_ALL_MAX, Math.max(1, limit));
    switch (shelf) {
      case "recentlyPlayed":
        return this.recentlyPlayed(n);
      case "jumpBackIn":
        return this.jumpBackIn(n).map((album) => ({ kind: "album", album }));
      case "playlists":
        return this.playlists(n).map((playlist) => ({ kind: "playlist", playlist }));
      case "recentlyAdded":
        return this.recentlyAdded(n).map((album) => ({ kind: "album", album }));
      case "topArtists":
        return this.topArtists(n).map((artist) => ({ kind: "artist", artist }));
    }
  }
}
