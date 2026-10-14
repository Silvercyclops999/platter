/** Home page decisions: chips, which shelves show, the time-of-day heading. */
import type { CachedLibrary, HomeData, HomeItem, HomeShelfId } from "../lib/types";

export type HomeFilter = "all" | "albums" | "artists" | "playlists";

export const HOME_FILTERS: { id: HomeFilter; label: string }[] = [
  { id: "all", label: "All" },
  { id: "albums", label: "Albums" },
  { id: "artists", label: "Artists" },
  { id: "playlists", label: "Playlists" },
];

const FILTER_KIND: Record<Exclude<HomeFilter, "all">, HomeItem["kind"]> = {
  albums: "album",
  artists: "artist",
  playlists: "playlist",
};

/** Shelves in their fixed order. */
export const HOME_SHELVES: { id: HomeShelfId; title: string }[] = [
  { id: "recentlyPlayed", title: "Recently played" },
  { id: "jumpBackIn", title: "Jump back in" },
  { id: "playlists", title: "Your playlists" },
  { id: "recentlyAdded", title: "Recently added" },
  { id: "topArtists", title: "Your top artists" },
];

export function shelfTitle(id: HomeShelfId): string {
  return HOME_SHELVES.find((s) => s.id === id)?.title ?? "";
}

/** One shelf's items, all as Home items. */
export function shelfItems(data: HomeData, id: HomeShelfId): HomeItem[] {
  switch (id) {
    case "recentlyPlayed":
      return data.recentlyPlayed;
    case "jumpBackIn":
      return data.jumpBackIn.map((album) => ({ kind: "album", album }));
    case "playlists":
      return data.playlists.map((playlist) => ({ kind: "playlist", playlist }));
    case "recentlyAdded":
      return data.recentlyAdded.map((album) => ({ kind: "album", album }));
    case "topArtists":
      return data.topArtists.map((artist) => ({ kind: "artist", artist }));
  }
}

export function filterItems(items: HomeItem[], filter: HomeFilter): HomeItem[] {
  return filter === "all" ? items : items.filter((i) => i.kind === FILTER_KIND[filter]);
}

/** The shelves to show for a chip: fixed order, narrowed to its type, empty ones left out. */
export function visibleShelves(data: HomeData, filter: HomeFilter): { id: HomeShelfId; title: string; items: HomeItem[] }[] {
  return HOME_SHELVES.map((s) => ({ ...s, items: filterItems(shelfItems(data, s.id), filter) })).filter((s) => s.items.length > 0);
}

/** Tiles in the shortcut grid, Liked Songs included. */
export const GRID_TILES = 8;

/** Liked Songs leads the grid on All and Playlists once something is liked. */
export function showsLikedTile(filter: HomeFilter, likedCount: number, enabled: boolean): boolean {
  return enabled && likedCount > 0 && (filter === "all" || filter === "playlists");
}

/** The shortcut grid's tiles for a chip (none when the setting is off), one fewer when Liked Songs leads. */
export function gridItems(data: HomeData, filter: HomeFilter, enabled: boolean, liked = false): HomeItem[] {
  return enabled ? filterItems(data.shortcuts, filter).slice(0, GRID_TILES - (liked ? 1 : 0)) : [];
}

/** Whether a Home item has anything stored on this computer. */
export function playableOffline(item: HomeItem, lib: CachedLibrary): boolean {
  switch (item.kind) {
    case "album":
      return lib.albums.some((a) => a.id === item.album.id);
    case "artist":
      return lib.artistIds.includes(item.artist.id);
    case "playlist":
      return lib.playlistIds.includes(item.playlist.id);
  }
}

/**
 * Home while the server can't be reached: only what can still play. Items
 * with nothing on this computer leave (they come back on reconnect).
 */
export function offlineHome(data: HomeData, lib: CachedLibrary): HomeData {
  const keep = (i: HomeItem) => playableOffline(i, lib);
  const albums = new Set(lib.albums.map((a) => a.id));
  return {
    shortcuts: data.shortcuts.filter(keep),
    recentlyPlayed: data.recentlyPlayed.filter(keep),
    jumpBackIn: data.jumpBackIn.filter((a) => albums.has(a.id)),
    playlists: data.playlists.filter((p) => lib.playlistIds.includes(p.id)),
    recentlyAdded: data.recentlyAdded.filter((a) => albums.has(a.id)),
    topArtists: data.topArtists.filter((a) => lib.artistIds.includes(a.id)),
  };
}

/** The screen-reader heading, by time of day. */
export function greeting(d = new Date()): string {
  const h = d.getHours();
  if (h < 5) return "Good night";
  if (h < 12) return "Good morning";
  if (h < 18) return "Good afternoon";
  return "Good evening";
}

/**
 * Opacity of the pinned chips' backdrop for a scroll offset (0 to 1): the
 * same fade as the main header's bar, so the two read as one strip.
 */
export function chipsAlpha(scrollTop: number): number {
  return Math.min(1, Math.max(0, (scrollTop - 16) / 100));
}
