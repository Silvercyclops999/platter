/** Data, recents and menus shared by the library sidebar, its rail and wide mode. */
import { useEffect, useMemo } from "react";
import type { MenuItem } from "../stores/ui";
import { addToPlaylistItem, albumSongs, playlistEditItems } from "../components/PlaylistPicker";
import { shareItem } from "../components/ShareMenu";
import { createPlaylist } from "../stores/playlists";
import { useLikes } from "../stores/likes";
import type { Playlist } from "../lib/types";
import { useApp } from "../stores/app";
import {
  LIB_KINDS,
  effectiveSort,
  libraryEntries,
  subtitle,
  swapInOrder,
  toEntries,
  useLibraryView,
  type LibEntry,
  type LibFilter,
  type LibKind,
  type LibSort,
  type LibSub,
} from "../stores/library";
import { usePins } from "../stores/pins";
import { player, usePlayer } from "../stores/player";
import { nav, useCurrentEntry } from "../stores/router";

/** Load the library items, and again after every library change. */
export function useLibraryLoader() {
  const version = useApp((s) => s.libraryVersion);
  useEffect(() => {
    void useLibraryView.getState().loadItems();
  }, [version]);
}

/** Opening an item, or starting to play it, moves it up the Recents sort. */
export function useLibraryRecents() {
  const r = useCurrentEntry().route;
  const ctx = usePlayer((s) => s.state?.queue.context);
  const ctxKey = ctx && (ctx.kind === "album" || ctx.kind === "artist" || ctx.kind === "playlist") && ctx.id ? `${ctx.kind}:${ctx.id}` : null;
  useEffect(() => {
    if (r.name === "album" || r.name === "artist" || r.name === "playlist") useLibraryView.getState().touch(r.name, r.id);
  }, [r]);
  useEffect(() => {
    if (!ctxKey) return;
    const [kind, id] = ctxKey.split(":") as [LibKind, string];
    useLibraryView.getState().touch(kind, id);
  }, [ctxKey]);
}

export interface LibraryModel {
  /** Every row (pins first), before filter and search. */
  all: LibEntry[];
  /** What to show. */
  shown: LibEntry[];
  /** Types the library has (the chips). */
  kinds: LibKind[];
  filter: LibFilter;
  sub: LibSub | null;
  sort: LibSort;
  loading: boolean;
}

/** Pins (kept fresh from the library) followed by everything else. */
export function mergePins(entries: LibEntry[], pins: ReturnType<typeof usePins.getState>["pins"]): LibEntry[] {
  const key = (k: string, id: string) => `${k}:${id}`;
  const byKey = new Map(entries.map((e) => [key(e.kind, e.id), e]));
  const pinned = pins.map((p): LibEntry => {
    const live = byKey.get(key(p.kind, p.id));
    if (live) return { ...live, pinned: true };
    return {
      kind: p.kind,
      id: p.id,
      title: p.title,
      sortTitle: p.title,
      creator: null,
      creatorId: null,
      thumb: p.thumb,
      smart: false,
      format: null,
      addedAt: null,
      playedAt: null,
      pinned: true,
      unavailable: true,
    };
  });
  const pinnedKeys = new Set(pins.map((p) => key(p.kind, p.id)));
  return [...pinned, ...entries.filter((e) => !pinnedKeys.has(key(e.kind, e.id)))];
}

/** The Liked Songs row: a playlist owned by you, pinned first. */
export function likedEntry(owner: string | null): LibEntry {
  return {
    kind: "playlist",
    id: "liked",
    title: "Liked Songs",
    sortTitle: "Liked Songs",
    creator: owner,
    creatorId: null,
    thumb: null,
    smart: false,
    format: null,
    addedAt: null,
    playedAt: null,
    pinned: true,
    liked: true,
  };
}

export function useLibraryModel(query: string): LibraryModel {
  const items = useLibraryView((s) => s.items);
  const filter = useLibraryView((s) => s.filter);
  const sub = useLibraryView((s) => s.sub);
  const sort = useLibraryView((s) => s.sort);
  const recents = useLibraryView((s) => s.recents);
  const customOrder = useLibraryView((s) => s.customOrder);
  const owner = useApp((s) => s.auth?.username ?? null);
  const pins = usePins((s) => s.pins);

  const all = useMemo(
    () => (items === null ? [] : [likedEntry(owner), ...mergePins(toEntries(items, owner), pins)]),
    [items, owner, pins],
  );
  const kinds = items === null ? LIB_KINDS : LIB_KINDS.filter((k) => all.some((e) => e.kind === k));
  // A saved filter for a type that's no longer there shows everything.
  const activeFilter: LibFilter = filter !== "all" && !kinds.includes(filter) ? "all" : filter;
  const activeSub = activeFilter === "all" ? null : sub;
  const shown = useMemo(
    () => libraryEntries(all, { filter: activeFilter, sub: activeSub, sort, query, recents, customOrder }),
    [all, activeFilter, activeSub, sort, query, recents, customOrder],
  );
  return { all, shown, kinds, filter: activeFilter, sub: activeSub, sort: effectiveSort(sort, activeFilter), loading: items === null };
}

export function playEntry(e: LibEntry) {
  const ctx = usePlayer.getState().state?.queue.context;
  if (e.liked) return ctx?.kind === "liked" ? player.toggle() : player.playLiked();
  if (ctx?.kind === e.kind && ctx.id === e.id) return player.toggle();
  useLibraryView.getState().touch(e.kind, e.id);
  return e.kind === "album" ? player.playAlbum(e.id) : e.kind === "artist" ? player.playArtist(e.id) : player.playPlaylist(e.id);
}

export function openEntry(e: LibEntry) {
  if (e.liked) return nav.go({ name: "liked" });
  nav.go({ name: e.kind, id: e.id });
}

/** Move a playlist one place in the custom order, among the playlists `shown`. */
export function movePlaylist(all: LibEntry[], shown: LibEntry[], id: string, by: -1 | 1) {
  const visible = shown.filter((e) => e.kind === "playlist" && !e.pinned).map((e) => e.id);
  const i = visible.indexOf(id);
  const other = visible[i + by];
  if (i < 0 || other === undefined) return;
  const s = useLibraryView.getState();
  const order = libraryEntries(all, { filter: "playlist", sort: "custom", query: "", recents: s.recents, customOrder: s.customOrder })
    .filter((e) => !e.pinned)
    .map((e) => e.id);
  s.setCustomOrder(swapInOrder(order, id, other));
}

/** Right-click menu for a row: the Phase 1 actions for its type. */
export function entryMenu(e: LibEntry, opts: { playing: boolean; move?: { up: boolean; down: boolean; run: (by: -1 | 1) => void } }): MenuItem[] {
  const pin: MenuItem = {
    label: e.pinned ? "Unpin" : "Pin",
    onSelect: () => usePins.getState().toggle({ kind: e.kind, id: e.id, title: e.title, subtitle: subtitle(e, false), thumb: e.thumb }),
  };
  if (e.unavailable) return [pin];
  const play: MenuItem = { label: opts.playing ? "Pause" : "Play", onSelect: () => void playEntry(e) };
  if (e.liked)
    return [
      play,
      { label: "Add to queue", onSelect: () => void player.addToQueue([...useLikes.getState().ids].map((id) => ({ id, thumb: null }))) },
      { label: "Create playlist", separator: true, onSelect: () => void createPlaylist() },
    ];
  const items: MenuItem[] = [play];
  if (e.kind === "album") {
    items.push({ label: "Add to queue", onSelect: () => void player.queueAlbum(e.id) });
    items.push(addToPlaylistItem(albumSongs(e.id)));
    if (e.creatorId) {
      const artistId = e.creatorId;
      items.push({ label: "Go to artist", onSelect: () => nav.go({ name: "artist", id: artistId }) });
    }
  }
  if (e.kind === "playlist") {
    items.push({ label: "Add to queue", onSelect: () => void player.queuePlaylist(e.id) });
    const known = useApp.getState().playlists?.find((p) => p.id === e.id);
    const pl: Playlist = known ?? { id: e.id, title: e.title, smart: e.smart, trackCount: 0, durationMs: 0, thumb: e.thumb ?? null };
    items.push(...playlistEditItems(pl));
    items.push({ label: "Create playlist", separator: true, onSelect: () => void createPlaylist() });
  }
  items.push(shareItem(e.kind, e.id, e.thumb));
  items.push({ ...pin, separator: true });
  if (opts.move) {
    const m = opts.move;
    items.push({ label: "Move up", separator: true, disabled: !m.up, onSelect: () => m.run(-1) });
    items.push({ label: "Move down", disabled: !m.down, onSelect: () => m.run(1) });
  }
  return items;
}
