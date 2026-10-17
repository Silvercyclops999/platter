/** Data, recents and menus shared by the library sidebar, its rail and wide mode. */
import { FolderPlus } from "lucide-react";
import { useEffect, useMemo } from "react";
import { useUi, type MenuItem } from "../stores/ui";
import { addToPlaylistItem, albumSongs, playlistEditItems } from "../components/PlaylistPicker";
import { shareItem } from "../components/ShareMenu";
import { createPlaylist } from "../stores/playlists";
import { bookmarkPageItem, deleteSmartPlaylist, editSmartPlaylist } from "../stores/curate";
import { useSettings } from "../stores/settings";
import { backend } from "../lib/backend";
import { downloadMenuItems, useDownloads } from "../stores/downloads";
import { sendToDeviceItem } from "../components/DeviceMenu";
import { startSessionItem, useSocial } from "../stores/social";
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
import { useOwnedIds } from "../stores/physical";
import { usePins } from "../stores/pins";
import { folders, useFolders, withFolders } from "../stores/folders";
import { folderMenu, moveToFolderItem } from "./FolderMenus";
import { player, usePlayer } from "../stores/player";
import { nav } from "../stores/router";
import { leavePlaylist, openCircleDialog, playBlend, playShared, setInLibrary } from "../stores/circle";

/** Load the library items, and again after every library change. */
export function useLibraryLoader() {
  const version = useApp((s) => s.libraryVersion);
  useEffect(() => {
    void useLibraryView.getState().loadItems();
  }, [version]);
}

/** Starting to play an item moves it up the Recents sort. Only opening it
 * doesn't, so the list doesn't jump under the pointer. */
export function useLibraryRecents() {
  const ctx = usePlayer((s) => s.state?.queue.context);
  const ctxKey = ctx && (ctx.kind === "album" || ctx.kind === "artist" || ctx.kind === "playlist") && ctx.id ? `${ctx.kind}:${ctx.id}` : null;
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
  /** The Downloaded chip: shown when anything is downloaded; on by itself while offline. */
  downloaded: { available: boolean; on: boolean };
  /** The folder the list is drilled into. */
  drill: import("../lib/types").LibraryFolder | null;
  /** The albums' "On the shelf" chip shows (something's owned and the setting is on). */
  shelf: boolean;
}

/** Pins (kept fresh from the library) followed by everything else. */
export function mergePins(entries: LibEntry[], all: ReturnType<typeof usePins.getState>["pins"]): LibEntry[] {
  const key = (k: string, id: string) => `${k}:${id}`;
  const byKey = new Map(entries.map((e) => [key(e.kind, e.id), e]));
  // Folder pins show where folders do (see withFolders).
  const pins = all.filter((p): p is typeof p & { kind: LibEntry["kind"] } => p.kind !== "folder");
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

/** The Bookmarks row: pinned after Liked Songs while its setting is on. */
export function bookmarksEntry(): LibEntry {
  return {
    kind: "playlist",
    id: "bookmarks",
    title: "Bookmarks",
    sortTitle: "Bookmarks",
    creator: null,
    creatorId: null,
    thumb: null,
    smart: false,
    format: null,
    addedAt: null,
    playedAt: null,
    pinned: true,
    bookmarks: true,
  };
}

/** The Stats row: pinned after Bookmarks while the Stats page is on. */
export function statsEntry(): LibEntry {
  return {
    kind: "playlist",
    id: "stats",
    title: "Stats",
    sortTitle: "Stats",
    creator: null,
    creatorId: null,
    thumb: null,
    smart: false,
    format: null,
    addedAt: null,
    playedAt: null,
    pinned: true,
    stats: true,
  };
}

/** `drillable`: the list follows the folder the sidebar is drilled into (not the rail). */
export function useLibraryModel(query: string, drillable = true): LibraryModel {
  const items = useLibraryView((s) => s.items);
  const filter = useLibraryView((s) => s.filter);
  const sub = useLibraryView((s) => s.sub);
  const sort = useLibraryView((s) => s.sort);
  const recents = useLibraryView((s) => s.recents);
  const customOrder = useLibraryView((s) => s.customOrder);
  const owner = useApp((s) => s.auth?.username ?? null);
  const pinsOn = useSettings((s) => s.settings?.pinsInLibrary !== false);
  const allPins = usePins((s) => s.pins);
  const pins = useMemo(() => (pinsOn ? allPins : []), [pinsOn, allPins]);
  const dlItems = useDownloads((s) => s.status?.items);
  const followIds = useSocial((s) => s.followIds);
  const ownedIds = useOwnedIds();
  const showBookmarks = useSettings((s) => s.settings?.bookmarksInLibrary !== false);
  const showStats = useSettings((s) => s.settings?.statsPage !== false);
  const dlChoice = useLibraryView((s) => s.downloaded);
  const cachedOnly = useApp((s) => !!s.health?.cachedOnly);
  // Going offline or back online hands the chip back to the connection.
  useEffect(() => useLibraryView.getState().setDownloaded(null), [cachedOnly]);

  const all = useMemo(() => {
    if (items === null) return [];
    const keys = new Set((dlItems ?? []).map((i) => `${i.kind}:${i.kind === "liked" ? "" : i.id}`));
    const mark = (e: LibEntry): LibEntry => {
      const on = keys.has(e.liked ? "liked:" : `${e.kind}:${e.id}`);
      const followed = e.kind === "artist" && followIds.has(e.id);
      const owned = e.kind === "album" && !!ownedIds?.has(e.id);
      return on || followed || owned
        ? { ...e, ...(on ? { downloaded: true } : {}), ...(followed ? { followed: true } : {}), ...(owned ? { owned: true } : {}) }
        : e;
    };
    const top = [likedEntry(owner), ...(showBookmarks ? [bookmarksEntry()] : []), ...(showStats ? [statsEntry()] : [])];
    return [...top, ...mergePins(toEntries(items, owner), pins)].map(mark);
  }, [items, owner, pins, dlItems, followIds, ownedIds, showBookmarks, showStats]);
  const shelf = !!ownedIds && all.some((e) => e.owned);
  const dlAvailable = all.some((e) => e.downloaded);
  const downloadedOnly = dlChoice ?? (cachedOnly && dlAvailable);
  const kinds = items === null ? LIB_KINDS : LIB_KINDS.filter((k) => all.some((e) => e.kind === k));
  // A saved filter for a type that's no longer there shows everything.
  const activeFilter: LibFilter = filter !== "all" && !kinds.includes(filter) ? "all" : filter;
  // The shelf chip saved from before, with nothing on the shelf now, shows every album.
  const activeSub = activeFilter === "all" || (sub === "shelf" && !shelf) ? null : sub;
  const tree = useFolders((s) => s.tree);
  const drill = useFolders((s) => s.drill);
  const expanded = useFolders((s) => s.expanded);
  // Folders group playlists while nothing narrows the list to albums,
  // artists, smart playlists, downloads or a search.
  const grouping = !query.trim() && (activeFilter === "all" || activeFilter === "playlist") && activeSub !== "smart" && !downloadedOnly;
  const shown = useMemo(
    () =>
      withFolders(libraryEntries(all, { filter: activeFilter, sub: activeSub, sort, query, recents, customOrder, downloadedOnly }), tree, {
        active: grouping,
        drill: drillable ? drill : null,
        expanded,
        pins,
      }),
    [pins, all, activeFilter, activeSub, sort, query, recents, customOrder, downloadedOnly, tree, drill, expanded, grouping, drillable],
  );
  return {
    all,
    shown,
    kinds,
    filter: activeFilter,
    sub: activeSub,
    sort: effectiveSort(sort, activeFilter),
    loading: items === null,
    downloaded: { available: dlAvailable, on: downloadedOnly },
    drill: grouping && drillable ? (tree.folders.find((f) => f.id === drill) ?? null) : null,
    shelf,
  };
}

export function playEntry(e: LibEntry) {
  if (e.folder) return;
  if (e.bookmarks) return nav.go({ name: "bookmarks" });
  if (e.stats) return nav.go({ name: "stats" });
  if (e.platter) return backend.smartPage(e.id).then((pg) => player.playTracks(pg.tracks.map((t) => t.id)), () => {});
  if (e.blend) return playBlend(e.id);
  if (e.shared) return playShared(e.id);
  const ctx = usePlayer.getState().state?.queue.context;
  if (e.liked) return ctx?.kind === "liked" ? player.toggle() : player.playLiked();
  if (ctx?.kind === e.kind && ctx.id === e.id) return player.toggle();
  useLibraryView.getState().touch(e.kind, e.id);
  return e.kind === "album" ? player.playAlbum(e.id) : e.kind === "artist" ? player.playArtist(e.id) : player.playPlaylist(e.id);
}

export function openEntry(e: LibEntry) {
  if (e.folder) {
    if (e.folder.nested) return folders.toggleExpanded(e.folder.id);
    // From the rail, the sidebar opens to show the folder.
    if (useUi.getState().sidebarCollapsed) useUi.getState().setSidebar({ collapsed: false });
    return folders.open(e.folder.id);
  }
  if (e.liked) return nav.go({ name: "liked" });
  if (e.bookmarks) return nav.go({ name: "bookmarks" });
  if (e.stats) return nav.go({ name: "stats" });
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
  if (e.folder) return folderMenu(e.folder.id);
  const pin: MenuItem = {
    label: e.pinned ? "Unpin" : "Pin",
    onSelect: () => usePins.getState().toggle({ kind: e.kind, id: e.id, title: e.title, subtitle: subtitle(e, false), thumb: e.thumb }),
  };
  if (e.unavailable) return [pin];
  const play: MenuItem = { label: opts.playing ? "Pause" : "Play", onSelect: () => void playEntry(e) };
  if (e.bookmarks) return [{ label: "Open", onSelect: () => nav.go({ name: "bookmarks" }) }];
  if (e.stats) return [{ label: "Open", onSelect: () => nav.go({ name: "stats" }) }];
  if (e.platter)
    return [
      play,
      { label: "Edit rules", onSelect: () => void editSmartPlaylist(e.id, e.title) },
      bookmarkPageItem("smart", e.id, e.title),
      { label: "Delete", separator: true, onSelect: () => void deleteSmartPlaylist(e.id) },
    ];
  if (e.blend)
    return [
      play,
      { label: "Add to queue", onSelect: () => void backend.blendPage(e.id).then((p) => player.addToQueue(p.rows.map((r) => r.track))) },
      { label: "Leave Blend", separator: true, onSelect: () => openCircleDialog({ kind: "leaveBlend", id: e.id, title: e.title }) },
      { ...pin, separator: true },
    ];
  if (e.shared)
    return [
      play,
      ...(e.shared.gone ? [] : [{ label: "Playlist permissions", onSelect: () => openCircleDialog({ kind: "permissions", playlistId: e.id }) }]),
      { label: "Remove from Your Library", onSelect: () => void setInLibrary(e.id, false) },
      e.shared.gone ? { label: "Remove", onSelect: () => void leavePlaylist(e.id) } : { label: "Leave playlist", onSelect: () => openCircleDialog({ kind: "leave", playlistId: e.id }) },
      { ...pin, separator: true },
    ];
  if (e.liked)
    return [
      play,
      { label: "Add to queue", onSelect: () => void player.addToQueue([...useLikes.getState().ids].map((id) => ({ id, thumb: null }))) },
      ...downloadMenuItems({ kind: "liked", id: "", title: "Liked Songs" }),
      sendToDeviceItem({ kind: "liked", id: "", title: "Liked Songs" }),
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
    items.push(moveToFolderItem({ kind: "playlist", id: e.id, title: e.title }));
    items.push({ label: "Create playlist", separator: true, onSelect: () => void createPlaylist() });
    items.push({ label: "Create folder", icon: FolderPlus, onSelect: () => void folders.create(null) });
  }
  if (e.kind === "album" || e.kind === "playlist") items.push(...downloadMenuItems({ kind: e.kind, id: e.id, title: e.title }));
  items.push(sendToDeviceItem({ kind: e.kind, id: e.id, title: e.title }));
  if (e.kind === "album" || e.kind === "playlist") items.push(startSessionItem(() => void playEntry(e)));
  items.push(shareItem(e.kind, e.id, e.thumb));
  items.push({ ...pin, separator: true });
  if (opts.move) {
    const m = opts.move;
    items.push({ label: "Move up", separator: true, disabled: !m.up, onSelect: () => m.run(-1) });
    items.push({ label: "Move down", disabled: !m.down, onSelect: () => m.run(1) });
  }
  return items;
}
