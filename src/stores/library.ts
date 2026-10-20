/**
 * Library sidebar: the items (every playlist, album and artist), the type
 * filter, sort, views and "recently opened" times. The view settings are
 * app-only and per device (localStorage), like pins.
 */
import { create } from "zustand";
import { backend } from "../lib/backend";
import type { LibraryItem } from "../lib/types";
import { idVariants, plainId } from "../lib/ids";

export type LibKind = "playlist" | "album" | "artist";
export type LibFilter = LibKind | "all";
/** Sub-chips: regular or smart playlists; followed artists; albums on the shelf. */
export type LibSub = "byYou" | "smart" | "followed" | "shelf";
export type LibSort = "recents" | "added" | "alpha" | "creator" | "custom";
export type LibView = "list" | "compactList" | "compactGrid" | "grid" | "shelf";

export const LIB_KINDS: LibKind[] = ["playlist", "album", "artist"];
/** The sidebar's views. */
export const LIB_VIEWS: LibView[] = ["compactList", "list", "compactGrid", "grid"];
/** Wide mode adds the record shelf. */
export const WIDE_VIEWS: LibView[] = [...LIB_VIEWS, "shelf"];
export const KIND_LABEL: Record<LibKind, string> = { playlist: "Playlists", album: "Albums", artist: "Artists" };
export const SUB_LABEL: Record<LibSub, string> = { byYou: "By you", smart: "Smart", followed: "Followed", shelf: "On the shelf" };
export const SUB_CHIPS: Record<LibKind, LibSub[]> = { playlist: ["byYou", "smart"], album: ["shelf"], artist: ["followed"] };
export const SORT_LABEL: Record<LibSort, string> = {
  recents: "Recents",
  added: "Recently added",
  alpha: "Alphabetical",
  creator: "Creator",
  custom: "Custom order",
};
export const VIEW_LABEL: Record<LibView, string> = {
  compactList: "Compact list",
  list: "List",
  compactGrid: "Compact grid",
  grid: "Grid",
  shelf: "Shelf",
};

const KEY = "platter.library-view.v1"; // gitleaks:allow (localStorage key name, not a secret)
const MAX_RECENTS = 300;

interface Saved {
  filter: LibFilter;
  sub: LibSub | null;
  sort: LibSort;
  /** Sidebar view. */
  view: LibView;
  /** Wide mode keeps its own view. */
  wideView: LibView;
  /** Playlist ids in the user's own order (the Custom order sort). */
  customOrder: string[];
  /** "kind:id" → when it was last opened or played (ms). */
  recents: Record<string, number>;
}

const DEFAULTS: Saved = { filter: "all", sub: null, sort: "recents", view: "list", wideView: "grid", customOrder: [], recents: {} };

const isView = (v: unknown): v is LibView => typeof v === "string" && (LIB_VIEWS as string[]).includes(v);
const isWideView = (v: unknown): v is LibView => typeof v === "string" && (WIDE_VIEWS as string[]).includes(v);
const isSort = (v: unknown): v is LibSort => typeof v === "string" && v in SORT_LABEL;

/** Read the saved settings, dropping anything unknown (older versions saved view "compact"). */
export function loadLibraryView(raw: string | null): Saved {
  if (!raw) return { ...DEFAULTS };
  try {
    const p = JSON.parse(raw) as Partial<Record<keyof Saved, unknown>>;
    const filter = p.filter === "all" || LIB_KINDS.includes(p.filter as LibKind) ? (p.filter as LibFilter) : "all";
    const sub = filter !== "all" && SUB_CHIPS[filter].includes(p.sub as LibSub) ? (p.sub as LibSub) : null;
    return {
      filter,
      sub,
      sort: isSort(p.sort) ? p.sort : DEFAULTS.sort,
      view: p.view === "compact" ? "compactList" : isView(p.view) ? p.view : DEFAULTS.view,
      wideView: isWideView(p.wideView) ? p.wideView : DEFAULTS.wideView,
      customOrder: Array.isArray(p.customOrder) ? p.customOrder.filter((x): x is string => typeof x === "string") : [],
      recents: p.recents && typeof p.recents === "object" ? (p.recents as Record<string, number>) : {},
    };
  } catch {
    return { ...DEFAULTS };
  }
}

function load(): Saved {
  try {
    return loadLibraryView(localStorage.getItem(KEY));
  } catch {
    return { ...DEFAULTS };
  }
}

function save(s: Saved) {
  try {
    localStorage.setItem(KEY, JSON.stringify(s));
  } catch {
    /* storage unavailable: this session only */
  }
}

interface LibraryViewState extends Saved {
  /** Every playlist, album and artist; null until first loaded. */
  items: LibraryItem[] | null;
  loadItems: () => Promise<void>;
  /** The last load failed with nothing loaded before it (the list shows why, with Try again). */
  loadFailed: unknown;
  setFilter: (f: LibFilter, sub?: LibSub | null) => void;
  setSub: (s: LibSub | null) => void;
  setSort: (s: LibSort) => void;
  setView: (v: LibView) => void;
  setWideView: (v: LibView) => void;
  setCustomOrder: (ids: string[]) => void;
  /** The Downloaded chip: on, off, or null to follow the connection (on while offline). Not saved. */
  downloaded: boolean | null;
  setDownloaded: (on: boolean | null) => void;
  /** Remember that an item was just opened or played (drives the Recents sort). */
  touch: (kind: LibKind, id: string, at?: number) => void;
}

const persist = (s: LibraryViewState) =>
  save({ filter: s.filter, sub: s.sub, sort: s.sort, view: s.view, wideView: s.wideView, customOrder: s.customOrder, recents: s.recents });

let loadSeq = 0;

export const useLibraryView = create<LibraryViewState>()((set, get) => {
  const update = (patch: Partial<Saved>) => {
    set(patch);
    persist(get());
  };
  return {
    ...load(),
    items: null,
    loadFailed: null,
    loadItems: async () => {
      const seq = ++loadSeq;
      try {
        const [plex, smart, circle] = await Promise.all([backend.libraryItems(), backend.smartList().catch(() => []), backend.circleState().catch(() => null)]);
        // Platter smart playlists, playlists shared with you and Blends sit with the Plex playlists.
        const items: LibraryItem[] = [
          ...plex,
          ...smart.map((s) => ({ kind: "playlist" as const, id: s.id, title: s.title, sortTitle: s.title, smart: true, platter: true, addedAt: s.updatedAt, playedAt: null })),
          ...(circle?.playlists ?? [])
            .filter((p) => p.role !== "owner" && p.inLibrary)
            .map((p) => ({ kind: "playlist" as const, id: p.id, title: p.title, sortTitle: p.title, smart: false, addedAt: null, playedAt: null, shared: { owner: p.owner.name, role: p.role, gone: p.gone } })),
          ...(circle?.blends ?? []).map((b) => ({ kind: "playlist" as const, id: b.id, title: b.title, sortTitle: b.title, smart: false, addedAt: null, playedAt: null, blend: true })),
        ];
        if (seq === loadSeq) set({ items, loadFailed: null });
      } catch (e) {
        if (seq === loadSeq && (get().items === null || get().loadFailed)) set({ items: [], loadFailed: e ?? true });
      }
    },
    setFilter: (filter, sub = null) => update({ filter, sub: filter === "all" ? null : sub }),
    setSub: (sub) => update({ sub }),
    setSort: (sort) => update({ sort }),
    setView: (view) => update({ view }),
    setWideView: (wideView) => update({ wideView }),
    setCustomOrder: (customOrder) => update({ customOrder }),
    downloaded: null,
    setDownloaded: (downloaded) => set({ downloaded }),
    touch: (kind, id, at = Date.now()) => {
      const entries = Object.entries({ ...get().recents, [`${kind}:${id}`]: at });
      // Keep the map bounded: drop the oldest.
      const kept = entries.length > MAX_RECENTS ? entries.sort((a, b) => b[1] - a[1]).slice(0, MAX_RECENTS) : entries;
      update({ recents: Object.fromEntries(kept) });
    },
  };
});

// ---------------------------------------------------------------- chips

export type Chip =
  | { kind: "clear" }
  | { kind: "type"; type: LibKind; selected: boolean }
  | { kind: "sub"; sub: LibSub; selected: boolean }
  | { kind: "downloaded"; selected: boolean };

/**
 * The chip row with the Downloaded chip added (when anything is downloaded).
 * It works alongside a type chip: on, the type's sub-chips hide.
 */
export function chipRowWithDownloads(
  filter: LibFilter,
  sub: LibSub | null,
  kinds: LibKind[],
  downloaded: { available: boolean; on: boolean },
  shelf = false,
): Chip[] {
  if (!downloaded.available && !downloaded.on) return chipRow(filter, sub, kinds, shelf);
  const dl: Chip = { kind: "downloaded", selected: downloaded.on };
  if (!downloaded.on) return [...chipRow(filter, sub, kinds, shelf), dl];
  if (filter === "all") return [{ kind: "clear" }, dl, ...kinds.map((type): Chip => ({ kind: "type", type, selected: false }))];
  return [{ kind: "clear" }, { kind: "type", type: filter, selected: true }, dl];
}

/**
 * The chip row. Unfiltered: one chip per type the library has. With a type
 * chosen: a clear chip, the selected type, then its sub-chips; with a
 * sub-chip chosen, only that one (pick it again to see its siblings).
 */
export function chipRow(filter: LibFilter, sub: LibSub | null, kinds: LibKind[] = LIB_KINDS, shelf = false): Chip[] {
  if (filter === "all") return kinds.map((type) => ({ kind: "type", type, selected: false }));
  // "On the shelf" only while the physical collection shows and something's owned.
  const subs = sub ? [sub] : SUB_CHIPS[filter].filter((s) => s !== "shelf" || shelf);
  return [
    { kind: "clear" },
    { kind: "type", type: filter, selected: true },
    ...subs.map((s): Chip => ({ kind: "sub", sub: s, selected: s === sub })),
  ];
}

/** The filter and sub-chip after a chip is pressed. One type, and one sub-chip, at a time. */
export function pressChip(chip: Chip, filter: LibFilter, sub: LibSub | null): { filter: LibFilter; sub: LibSub | null } {
  switch (chip.kind) {
    case "clear":
      return { filter: "all", sub: null };
    case "downloaded":
      return { filter, sub };
    case "type":
      // Pressing the selected type clears everything.
      return filter === chip.type ? { filter: "all", sub: null } : { filter: chip.type, sub: null };
    case "sub":
      return { filter, sub: sub === chip.sub ? null : chip.sub };
  }
}

/** Search field placeholder for the current filter. */
export function searchPlaceholder(filter: LibFilter): string {
  return filter === "all" ? "Search in Your Library" : `Search in ${KIND_LABEL[filter].toLowerCase()}`;
}

// ---------------------------------------------------------------- rows

export interface LibEntry {
  kind: LibKind;
  id: string;
  title: string;
  sortTitle: string;
  /** Album artist, or the playlist owner. */
  creator: string | null;
  /** Album artist id. */
  creatorId: string | null;
  thumb: string | null;
  smart: boolean;
  format: string | null;
  addedAt: number | null;
  playedAt: number | null;
  pinned: boolean;
  /** The album's cover colour from the server (albums only). */
  tint?: string | null;
  /** Songs on the album (albums only). */
  trackCount?: number | null;
  /** A pinned item that's no longer in the library. */
  unavailable?: boolean;
  /** Liked Songs (always pinned first; opens its own page). */
  liked?: boolean;
  /** Bookmarks (pinned after Liked Songs while its setting is on). */
  bookmarks?: boolean;
  /** The Stats page (pinned after Bookmarks while its setting is on). */
  stats?: boolean;
  /** A page pinned from Home's Explore row (Crates, Sound map). */
  page?: import("./pinnedPages").PinnablePage;
  /** A Platter smart playlist (kept and filled on this computer). */
  platter?: boolean;
  /** Someone else's playlist shared with you. */
  shared?: { role: string; gone: boolean };
  /** A Blend. */
  blend?: boolean;
  /** Kept on this computer (downloads). */
  downloaded?: boolean;
  /** An artist you follow. */
  followed?: boolean;
  /** An album you own on vinyl, CD, cassette or another format. */
  owned?: boolean;
  /** A library folder (Platter's own; Plex has none). */
  folder?: FolderInfo;
  /** How far in it sits under open folders (drilled-in lists). */
  depth?: number;
}

export interface FolderInfo {
  id: string;
  /** Playlists directly inside. */
  count: number;
  depth: number;
  /** Open in place (folders inside a drilled-in folder). */
  expanded: boolean;
  hasChildren: boolean;
  /** Inside the folder the sidebar is drilled into: it opens in place. */
  nested: boolean;
}

/** Library items as rows. Playlists are the signed-in user's own, so they're the creator. */
export function toEntries(items: LibraryItem[], owner: string | null): LibEntry[] {
  return items.map((i) => ({
    kind: i.kind,
    id: i.id,
    title: i.title,
    sortTitle: i.sortTitle || i.title,
    // A Platter smart playlist shows the "Platter" mark where the owner goes.
    creator: i.platter ? "Platter" : i.shared ? i.shared.owner : i.blend ? null : i.kind === "playlist" ? owner : (i.creator ?? null),
    creatorId: i.creatorId ?? null,
    thumb: i.thumb ?? null,
    smart: i.smart,
    format: i.format ?? null,
    addedAt: i.addedAt ?? null,
    playedAt: i.playedAt ?? null,
    pinned: false,
    ...(i.tint ? { tint: i.tint } : {}),
    ...(i.trackCount ? { trackCount: i.trackCount } : {}),
    ...(i.platter ? { platter: true } : {}),
    ...(i.shared ? { shared: { role: i.shared.role, gone: i.shared.gone } } : {}),
    ...(i.blend ? { blend: true } : {}),
  }));
}

/** "Playlist", "Smart playlist", "Album", "EP", "Single", "Artist". */
export function typeLabel(e: Pick<LibEntry, "kind" | "smart" | "format" | "folder" | "blend">): string {
  if (e.folder) return "Folder";
  if (e.blend) return "Blend";
  if (e.kind === "playlist") return e.smart ? "Smart playlist" : "Playlist";
  if (e.kind === "artist") return "Artist";
  return e.format === "ep" ? "EP" : e.format === "single" ? "Single" : "Album";
}

/**
 * Subtitle under the title: "Type · creator" unfiltered. Under a type filter
 * the type word goes: playlists show the owner, albums the artist, and
 * artists nothing.
 */
export function subtitle(e: LibEntry, filtered: boolean): string {
  if (e.bookmarks) return "Songs and pages";
  if (e.stats) return "Your listening";
  if (e.page) return e.page === "crates" ? "By label, producer and era" : "Songs that sound alike";
  if (e.unavailable) return "Not available";
  if (e.folder) return e.folder.count === 1 ? "1 playlist" : `${e.folder.count} playlists`;
  if (filtered) return e.kind === "artist" ? "" : (e.creator ?? "");
  const t = typeLabel(e);
  return e.kind !== "artist" && e.creator ? `${t} · ${e.creator}` : t;
}

// ---------------------------------------------------------------- search

/** Lower case, accents removed. */
export function fold(s: string): string {
  return s.normalize("NFD").replace(/\p{M}/gu, "").toLowerCase();
}

function words(s: string): string[] {
  return fold(s).split(/[^\p{L}\p{N}]+/u).filter(Boolean);
}

/**
 * Library search: every query word must start a word of the title or the
 * creator line, in any order. Case and accents don't matter; no typo
 * tolerance, and no matches from the middle of a word.
 */
export function matchesQuery(e: Pick<LibEntry, "title" | "creator">, query: string): boolean {
  const q = words(query);
  if (q.length === 0) return true;
  const w = words(`${e.title} ${e.creator ?? ""}`);
  return q.every((qw) => w.some((x) => x.startsWith(qw)));
}

// ---------------------------------------------------------------- sort

export interface EntryOpts {
  filter: LibFilter;
  sub?: LibSub | null;
  sort: LibSort;
  query: string;
  /** "kind:id" → last opened or played (ms). */
  recents: Record<string, number>;
  customOrder?: string[];
  /** Only downloaded items. */
  downloadedOnly?: boolean;
}

/** Custom order only exists under the Playlists filter; elsewhere it reads as Recents. */
export function effectiveSort(sort: LibSort, filter: LibFilter): LibSort {
  return sort === "custom" && filter !== "playlist" ? "recents" : sort;
}

const collator = new Intl.Collator(undefined, { sensitivity: "base", numeric: true });
const byTitle = (a: LibEntry, b: LibEntry) => collator.compare(a.sortTitle, b.sortTitle) || collator.compare(a.title, b.title);
const creatorKey = (e: LibEntry) => (e.kind === "artist" ? e.sortTitle : (e.creator ?? ""));

/** When the item was last opened or played here, or played anywhere (ms). */
export function recentAt(e: LibEntry, recents: Record<string, number>): number {
  const saved = idVariants(e.id).map((id) => recents[`${e.kind}:${id}`]).find((v) => v != null);
  return Math.max(saved ?? 0, (e.playedAt ?? 0) * 1000);
}

/**
 * What the library shows: pinned items first (in pin order, whatever the
 * sort), then the rest sorted, all narrowed by the type filter, sub-chip and
 * search text.
 */
export function libraryEntries(items: LibEntry[], opts: EntryOpts): LibEntry[] {
  const sub = opts.filter !== "all" && opts.sub && SUB_CHIPS[opts.filter].includes(opts.sub) ? opts.sub : null;
  const keep = (e: LibEntry) =>
    (opts.filter === "all" || e.kind === opts.filter) &&
    (sub === null || (sub === "followed" ? !!e.followed : sub === "shelf" ? !!e.owned : sub === "smart" ? e.smart : !e.smart && !e.shared && !e.blend)) &&
    (!opts.downloadedOnly || !!e.downloaded) &&
    matchesQuery(e, opts.query);
  const pinned = items.filter((e) => e.pinned && keep(e));
  const sort = effectiveSort(opts.sort, opts.filter);
  const order = new Map((opts.customOrder ?? []).map((id, i) => [id, i]));
  const rest = items
    .map((e, i) => ({ e, i }))
    .filter(({ e }) => !e.pinned && keep(e))
    .sort((a, b) => {
      switch (sort) {
        case "alpha":
          return byTitle(a.e, b.e);
        case "added":
          return (b.e.addedAt ?? -1) - (a.e.addedAt ?? -1) || byTitle(a.e, b.e);
        case "creator":
          return collator.compare(creatorKey(a.e), creatorKey(b.e)) || byTitle(a.e, b.e);
        case "custom": {
          // Playlists not placed yet follow the placed ones, by title.
          const oa = order.get(a.e.id) ?? order.get(plainId(a.e.id)) ?? Infinity;
          const ob = order.get(b.e.id) ?? order.get(plainId(b.e.id)) ?? Infinity;
          return oa === ob ? byTitle(a.e, b.e) : oa - ob;
        }
        default:
          return recentAt(b.e, opts.recents) - recentAt(a.e, opts.recents) || a.i - b.i;
      }
    })
    .map(({ e }) => e);
  return [...pinned, ...rest];
}

/**
 * The custom order with playlists `a` and `b` swapped. `order` is every
 * playlist in its current custom order, so moving within a narrowed list
 * keeps the others where they are.
 */
export function swapInOrder(order: string[], a: string, b: string): string[] {
  const i = order.indexOf(a);
  const j = order.indexOf(b);
  if (i < 0 || j < 0) return order;
  const next = [...order];
  [next[i], next[j]] = [next[j], next[i]];
  return next;
}
