/**
 * Console: three panes for big libraries. On the left a tree (sections,
 * artists with their albums, albums by year, labels, playlists and quick
 * filters); in the middle a dense table of songs with the columns you pick,
 * sortable by any of them; on the right a details pane (cover, tags, signal
 * path, credits) for the song you picked or the one playing. A one-line
 * transport runs along the bottom as a status bar.
 *
 * The table shows albums, playlists and Liked songs; every other page shows
 * as it does elsewhere. Columns, the sort and the pane widths are kept on
 * this device. The transport is the player bar's own, so names, keys and
 * tab stops don't change.
 */
import { ArrowDown, ArrowUp, Boxes, ChevronDown, ChevronRight, Disc3, Heart, History, House, ListMusic, PanelRight, Pause, Play, Settings as SettingsIcon, SlidersHorizontal, Tag, UsersRound, CalendarDays, X } from "lucide-react";
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type KeyboardEvent, type PointerEvent, type ReactNode } from "react";
import { create } from "zustand";
import { Art } from "../components/Art";
import { ArtistLinks, TextLink } from "../components/ArtistLinks";
import { Button, IconButton } from "../components/Button";
import { ExplicitMark } from "../components/FileTagBadges";
import { openContextMenu } from "../components/ContextMenu";
import { SaveButton } from "../components/SaveButton";
import { Skeleton } from "../components/Skeleton";
import { trackMenuItems } from "../components/TrackRow";
import { VirtualRows, type VirtualRowsHandle } from "../components/VirtualRows";
import { backend } from "../lib/backend";
import {
  CONSOLE_COLUMNS,
  CONSOLE_DEFAULT_COLUMNS,
  COLUMN_NAME,
  FILTER_NAME,
  PANE_LIMITS,
  clampPane,
  filterRows,
  formatCell,
  nextSort,
  parseSort,
  sortRows,
  type ConsoleColumn,
  type ConsoleFilter,
  type ConsoleRow,
  type Pane,
  type SortKey,
} from "../lib/consoleLayout";
import { formatDuration, formatLongDuration, plural, qualityLabel, trackArtistLine } from "../lib/format";
import type { Album, ArtistName, Crate, Track } from "../lib/types";
import { useLoad } from "../lib/useLoad";
import { useContextState } from "../pages/cards";
import { endSongDrag, startSongDrag, tracksDrag } from "../stores/drag";
import { fullWindow, useFullWindow } from "../stores/fullWindow";
import { useLibraryView, toEntries } from "../stores/library";
import { player, usePlayer } from "../stores/player";
import { nav, useCurrentEntry, type Route } from "../stores/router";
import { updateSettings, useSettings } from "../stores/settings";
import { useSignal } from "../stores/signalPath";
import { useLayout } from "../stores/theme";
import { useUi } from "../stores/ui";
import { useApp } from "../stores/app";
import { useCreditRows } from "./creditRows";
import { NowPlayingAnnouncer, PlayControls, SeekBar } from "./PlayerBar";
import { SearchBox } from "./SearchBox";
import { SignalPanel, stages, useStreamLabel } from "./SignalPath";
import { StatusDot } from "./StatusDot";
import { StripExtras } from "./TransportStrip";
import { openEntry, useLibraryLoader } from "./useLibrary";
import "./console.css";

// ---------------------------------------------------------------- state

interface ConsoleState {
  /** The song picked in the table (the details pane shows it, else what's playing). */
  selected: Track | null;
  filters: ConsoleFilter[];
  /** The details pane as a drawer, in a narrow window. */
  detailsOpen: boolean;
  /** Tree groups and artists that are open. */
  open: Record<string, boolean>;
  /** A pane width while its handle is being dragged. */
  live: Partial<Record<Pane, number>>;
}

export const useConsole = create<ConsoleState>()(() => ({ selected: null, filters: [], detailsOpen: false, open: { playlists: true }, live: {} }));

export function resetConsole() {
  useConsole.setState({ selected: null, filters: [], detailsOpen: false, open: { playlists: true }, live: {} });
}

/** The pane widths in use: a drag in progress, else the saved ones. */
export function usePaneWidths(): { tree: number; details: number } {
  const tree = useSettings((s) => s.settings?.consoleTree ?? PANE_LIMITS.tree.start);
  const details = useSettings((s) => s.settings?.consoleDetails ?? PANE_LIMITS.details.start);
  const live = useConsole((s) => s.live);
  return { tree: live.tree ?? tree, details: live.details ?? details };
}

// ---------------------------------------------------------------- what the table shows

export type ConsoleSource = { kind: "album"; id: string } | { kind: "playlist"; id: string } | { kind: "liked" };

/** The pages the table shows: an album (not its sleeve notes), a playlist, Liked songs. */
export function consoleSourceOf(r: Route): ConsoleSource | null {
  if (r.name === "album" && !r.tab) return { kind: "album", id: r.id };
  if (r.name === "playlist") return { kind: "playlist", id: r.id };
  if (r.name === "liked") return { kind: "liked" };
  return null;
}

interface Loaded {
  kind: string;
  title: string;
  meta: string;
  thumb: string | null;
  rows: ConsoleRow[];
  /** Play from a song, in the order shown when it isn't the list's own. */
  play: (startId?: string, ids?: string[]) => void;
}

function useSourceRows(src: ConsoleSource): { data: Loaded | undefined; error: boolean; retry: () => void } {
  const id = src.kind === "liked" ? "liked" : src.id;
  const { data, error, retry } = useLoad<Loaded>(async () => {
    if (src.kind === "album") {
      const p = await backend.albumPage(src.id);
      const year = p.album.year ?? null;
      return {
        kind: "Album",
        title: p.album.title,
        meta: [p.album.artistTitle, year, plural(p.tracks.length, "song")].filter(Boolean).join(", "),
        thumb: p.album.thumb,
        rows: p.tracks.map((track, index) => ({ track, year, index })),
        play: (start, ids) => (ids ? player.playTracks(ids, start) : player.playAlbum(src.id, start)),
      };
    }
    if (src.kind === "playlist") {
      const p = await backend.playlistPage(src.id);
      return {
        kind: "Playlist",
        title: p.playlist.title,
        meta: plural(p.tracks.length, "song"),
        thumb: p.playlist.thumb,
        rows: p.tracks.map((track, index) => ({ track, year: null, index })),
        play: (start, ids) => player.playPlaylist(src.id, start, undefined, ids),
      };
    }
    const p = await backend.likedPage();
    return {
      kind: "Your library",
      title: "Liked songs",
      meta: plural(p.tracks.length, "song"),
      thumb: null,
      rows: p.tracks.map((track, index) => ({ track, year: null, index })),
      play: (start, ids) => player.playLiked(start, ids),
    };
  }, [src.kind, id]);
  return { data, error: !!error, retry };
}

// ---------------------------------------------------------------- the table

const WIDTH: Record<"cover" | SortKey, string> = {
  cover: "32px",
  name: "minmax(160px, 2.4fr)",
  artist: "minmax(110px, 1.3fr)",
  album: "minmax(110px, 1.3fr)",
  year: "52px",
  format: "minmax(76px, 0.8fr)",
  length: "60px",
  plays: "60px",
};

/** The grid's column template for the shown columns. */
export function columnTemplate(cols: readonly ConsoleColumn[]): string {
  return ["cover", "name", ...cols].map((c) => WIDTH[c as keyof typeof WIDTH]).join(" ");
}

const NUMERIC: readonly SortKey[] = ["year", "length", "plays"];

function cellText(r: ConsoleRow, c: ConsoleColumn): string {
  const t = r.track;
  switch (c) {
    case "artist":
      return trackArtistLine(t);
    case "album":
      return t.albumTitle;
    case "year":
      return r.year ? String(r.year) : "";
    case "format":
      return formatCell(t);
    case "length":
      return formatDuration(t.durationMs);
    case "plays":
      return t.viewCount.toLocaleString();
  }
}

const ROW_HEIGHT = 32;

/** Picks which columns show, in the table's order. */
function columnsMenu(cols: readonly ConsoleColumn[], el: HTMLElement) {
  openContextMenu(
    "Table columns",
    CONSOLE_COLUMNS.map((c) => ({
      label: COLUMN_NAME[c],
      icon: null,
      checked: cols.includes(c),
      onSelect: () => void updateSettings({ consoleColumns: CONSOLE_COLUMNS.filter((x) => (x === c ? !cols.includes(c) : cols.includes(x))) }),
    })),
    el,
  );
}

function SkeletonRows({ template }: { template: string }) {
  return (
    <div className="console-skeleton" aria-hidden="true">
      {Array.from({ length: 10 }, (_, i) => (
        <div key={i} className="console-row console-row--skeleton" style={{ gridTemplateColumns: template }}>
          <Skeleton width={24} height={24} />
          <Skeleton width={`${50 + ((i * 17) % 40)}%`} height={11} />
        </div>
      ))}
    </div>
  );
}

/** The table page for an album, a playlist or Liked songs. */
export function ConsoleTable({ src }: { src: ConsoleSource }) {
  const chosen = useSettings((s) => s.settings?.consoleColumns ?? CONSOLE_DEFAULT_COLUMNS);
  const sort = useSettings((s) => s.settings?.consoleSort ?? "");
  const filters = useConsole((s) => s.filters);
  const selected = useConsole((s) => s.selected);
  const playingId = usePlayer((s) => s.state?.track?.id ?? null);
  const songDrags = useSettings((s) => s.settings?.dragAndDrop !== false);
  const { data, error, retry } = useSourceRows(src);
  const ctxKind = src.kind;
  const ctx = useContextState(ctxKind, src.kind === "liked" ? "liked" : src.id);
  // Years come with albums only: the column steps aside where nothing has one.
  const hasYear = !!data?.rows.some((r) => r.year !== null);
  const cols = chosen.filter((c) => c !== "year" || hasYear);
  const template = columnTemplate(cols);
  const rows = useMemo(() => sortRows(filterRows(data?.rows ?? [], filters), sort), [data, filters, sort]);
  const narrowed = !!parseSort(sort) || filters.length > 0;
  const ids = rows.map((r) => r.track.id);
  const total = rows.reduce((n, r) => n + r.track.durationMs, 0);
  const [focusKey, setFocusKey] = useState<number | null>(null);
  const grid = useRef<HTMLDivElement>(null);
  const virtual = useRef<VirtualRowsHandle>(null);
  const pending = useRef<number | null>(null);
  const columnsButton = useRef<HTMLSpanElement>(null);

  // A new list starts with nothing picked.
  useEffect(() => useConsole.setState({ selected: null }), [src.kind, src.kind === "liked" ? "" : src.id]);

  // Focus a row moved to with the keys, once it's on the page.
  useLayoutEffect(() => {
    const key = pending.current;
    if (key === null) return;
    const el = grid.current?.querySelector<HTMLElement>(`[data-row="${key}"]`);
    if (!el) return;
    pending.current = null;
    el.focus();
    el.scrollIntoView?.({ block: "nearest" });
  });

  const stop = focusKey !== null && rows.some((r) => r.index === focusKey) ? focusKey : (rows[0]?.index ?? null);
  const play = (start?: string) => data?.play(start, narrowed ? ids : undefined);
  const pick = (r: ConsoleRow) => {
    setFocusKey(r.index);
    useConsole.setState({ selected: r.track });
  };
  const move = (to: number) => {
    const r = rows[Math.max(0, Math.min(rows.length - 1, to))];
    if (!r) return;
    pick(r);
    pending.current = r.index;
    virtual.current?.scrollToIndex(rows.indexOf(r));
  };
  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const at = rows.findIndex((r) => r.index === stop);
    const step: Record<string, number> = { ArrowDown: 1, ArrowUp: -1, PageDown: 10, PageUp: -10 };
    if (e.key in step) {
      e.preventDefault();
      move(at + step[e.key]);
    } else if (e.key === "Home" || e.key === "End") {
      e.preventDefault();
      move(e.key === "Home" ? 0 : rows.length - 1);
    } else if (e.key === "Enter" && at >= 0) {
      e.preventDefault();
      play(rows[at].track.id);
    }
  };

  const header = (
    <header className="console-table__head">
      {data ? (
        <>
          <div className="console-table__title">
            <span className="console-table__kind">{data.kind}</span>
            <h1 className="console-table__name truncate">{data.title}</h1>
            <span className="console-table__meta truncate">{data.meta}</span>
          </div>
          <div className="console-table__actions">
            <IconButton
              label={ctx.playing ? "Pause" : `Play ${data.title}`}
              tooltip={ctx.playing ? "Pause" : "Play"}
              className="console-table__play"
              icon={ctx.playing ? <Pause size={16} fill="currentColor" /> : <Play size={16} fill="currentColor" />}
              onClick={() => (ctx.isContext && !narrowed ? player.toggle() : play())}
            />
            {src.kind === "album" && (
              <Button size="sm" variant="ghost" onClick={() => nav.go({ name: "album", id: src.id, tab: "sleeve" })}>
                Sleeve notes
              </Button>
            )}
            <span ref={columnsButton}>
              <Button size="sm" variant="ghost" aria-haspopup="menu" onClick={() => columnsMenu(chosen, columnsButton.current!.querySelector("button")!)}>
                Columns
                <ChevronDown size={14} aria-hidden="true" />
              </Button>
            </span>
          </div>
        </>
      ) : (
        <div className="console-table__title" aria-hidden="true">
          <Skeleton width={60} height={10} />
          <Skeleton width={220} height={20} style={{ marginTop: 6 }} />
        </div>
      )}
    </header>
  );

  let body: ReactNode;
  if (error && !data) {
    body = (
      <div className="console-state">
        <p>Couldn't load these songs.</p>
        <Button size="sm" onClick={retry}>
          Try again
        </Button>
      </div>
    );
  } else if (!data) {
    body = <SkeletonRows template={template} />;
  } else if (!data.rows.length) {
    body = <p className="console-state">No songs here yet.</p>;
  } else if (!rows.length) {
    body = (
      <div className="console-state">
        <p>No songs match these filters.</p>
        <Button size="sm" onClick={() => useConsole.setState({ filters: [] })}>
          Clear filters
        </Button>
      </div>
    );
  } else {
    body = (
      <VirtualRows
        count={rows.length}
        rowHeight={ROW_HEIGHT}
        spacer="div"
        scroller=".console-table__scroll"
        handle={virtual}
        renderRow={(i) => {
          const r = rows[i];
          const t = r.track;
          const on = selected?.id === t.id && stop === r.index;
          const now = playingId === t.id;
          return (
            <div
              key={r.index}
              role="row"
              data-row={r.index}
              aria-rowindex={i + 2}
              aria-selected={on}
              tabIndex={stop === r.index ? 0 : -1}
              className={`console-row ${on ? "is-selected" : ""} ${now ? "is-playing" : ""}`}
              style={{ gridTemplateColumns: template }}
              onClick={() => pick(r)}
              onFocus={() => setFocusKey(r.index)}
              onDoubleClick={() => play(t.id)}
              onContextMenu={(e) => {
                e.preventDefault();
                pick(r);
                openContextMenu(`Options for ${t.title}`, trackMenuItems(t), e);
              }}
              draggable={songDrags}
              onDragStart={songDrags ? (e) => void startSongDrag(e, tracksDrag([t])) : undefined}
              onDragEnd={songDrags ? endSongDrag : undefined}
            >
              <div role="gridcell" className="console-cell console-cell--cover">
                <Art path={t.thumb} size={24} />
                {now && <span className="console-row__now" aria-label="Playing" />}
              </div>
              <div role="gridcell" className="console-cell console-cell--name">
                <span className="truncate">{t.title}</span>
                <ExplicitMark trackId={t.id} />
              </div>
              {cols.map((c) => (
                <div key={c} role="gridcell" className={`console-cell console-cell--${c} ${NUMERIC.includes(c) ? "tabular" : ""}`}>
                  <span className="truncate">{cellText(r, c)}</span>
                </div>
              ))}
            </div>
          );
        }}
      />
    );
  }

  const sorted = parseSort(sort);
  return (
    <main id="main" className="main console-table" aria-label={data?.title ?? "Songs"} style={{ "--console-cols": template } as CSSProperties}>
      {header}
      <div className="console-table__scroll">
        <div
          ref={grid}
          className="console-grid"
          role="grid"
          aria-label={data ? `${data.title} songs` : "Songs"}
          aria-rowcount={rows.length + 1}
          aria-colcount={cols.length + 2}
          aria-busy={!data || undefined}
          onKeyDown={onKey}
        >
          <div role="row" aria-rowindex={1} className="console-row console-row--head" style={{ gridTemplateColumns: template }}>
            <div role="columnheader" className="console-cell console-cell--cover">
              <span className="sr-only">Cover</span>
            </div>
            {(["name", ...cols] as SortKey[]).map((c) => {
              const on = sorted?.key === c;
              return (
                <div key={c} role="columnheader" aria-sort={on ? (sorted!.desc ? "descending" : "ascending") : "none"} className={`console-cell console-cell--${c}`}>
                  <button type="button" className={`console-sort ${on ? "is-on" : ""} ${NUMERIC.includes(c) ? "is-numeric" : ""}`} onClick={() => void updateSettings({ consoleSort: nextSort(sort, c) })}>
                    {COLUMN_NAME[c]}
                    <span className="console-sort__mark" aria-hidden="true">
                      {on && (sorted!.desc ? <ArrowDown size={12} /> : <ArrowUp size={12} />)}
                    </span>
                  </button>
                </div>
              );
            })}
          </div>
          <div role="rowgroup">{body}</div>
        </div>
      </div>
      {data && data.rows.length > 0 && (
        <footer className="console-table__foot tabular">
          {narrowed && filters.length ? `${rows.length} of ${plural(data.rows.length, "song")}` : plural(rows.length, "song")}, {formatLongDuration(total)}
        </footer>
      )}
    </main>
  );
}

// ---------------------------------------------------------------- the tree

interface TreeNode {
  key: string;
  level: number;
  label: string;
  icon?: ReactNode;
  count?: number;
  /** The page it opens is the one showing. */
  on?: boolean;
  expanded?: boolean;
  checked?: boolean;
  parent?: string;
  activate: () => void;
  toggle?: () => void;
}

function useCrates(kind: "year" | "label"): Crate[] | null {
  const [crates, setCrates] = useState<Crate[] | null>(null);
  const version = useApp((s) => s.libraryVersion);
  useEffect(() => {
    let alive = true;
    backend.crates(kind).then(
      (l) => alive && setCrates(l.crates),
      () => alive && setCrates([]),
    );
    return () => {
      alive = false;
    };
  }, [kind, version]);
  return crates;
}

const setOpen = (key: string, on: boolean) => useConsole.setState((s) => ({ open: { ...s.open, [key]: on } }));

/** The tree's rows, flattened in order with their levels (open groups only). */
function useTreeNodes(): TreeNode[] {
  useLibraryLoader();
  const route = useCurrentEntry().route;
  const open = useConsole((s) => s.open);
  const filters = useConsole((s) => s.filters);
  const crates = useSettings((s) => s.settings?.showCrates !== false);
  const items = useLibraryView((s) => s.items);
  const owner = useApp((s) => s.auth?.username ?? null);
  const years = useCrates("year");
  const labels = useCrates("label");
  const [artists, setArtists] = useState<ArtistName[] | null>(null);
  const [albums, setAlbums] = useState<Record<string, Album[] | null>>({});

  useEffect(() => {
    if (!open.artists || artists) return;
    let alive = true;
    backend.artistNames().then(
      (a) => alive && setArtists([...a].sort((x, y) => x.title.localeCompare(y.title, undefined, { sensitivity: "base" }))),
      () => alive && setArtists([]),
    );
    return () => {
      alive = false;
    };
  }, [open.artists, artists]);

  const openArtist = (id: string, on: boolean) => {
    setOpen(`artist:${id}`, on);
    if (!on || albums[id] !== undefined) return;
    setAlbums((m) => ({ ...m, [id]: null }));
    backend.artistPage(id).then(
      (p) => setAlbums((m) => ({ ...m, [id]: p.sections.flatMap((s) => s.albums) })),
      () => setAlbums((m) => ({ ...m, [id]: [] })),
    );
  };

  const nodes: TreeNode[] = [];
  const go = (r: Route) => () => nav.go(r);
  const is = (r: Route) => JSON.stringify(r) === JSON.stringify(route);
  const section = (key: string, label: string, icon: ReactNode, r: Route, on = is(r)) => nodes.push({ key, level: 1, label, icon, on, activate: go(r) });
  section("home", "Home", <House size={15} />, { name: "home" });
  section("liked", "Liked songs", <Heart size={15} />, { name: "liked" });
  section("recent", "Recently added", <Disc3 size={15} />, { name: "homeShelf", shelf: "recentlyAdded" });
  section("history", "History", <History size={15} />, { name: "recents" });
  if (crates) section("crates", "Crates", <Boxes size={15} />, { name: "crates" }, route.name === "crates");

  const group = (key: string, label: string, icon: ReactNode, count?: number) => {
    const expanded = !!open[key];
    nodes.push({ key, level: 1, label, icon, count, expanded, activate: () => setOpen(key, !expanded), toggle: () => setOpen(key, !expanded) });
    return expanded;
  };
  const loading = (parent: string) => nodes.push({ key: `${parent}:loading`, level: 2, label: "Loading", parent, activate: () => {} });

  if (group("artists", "Artists", <UsersRound size={15} />, artists?.length)) {
    if (!artists) loading("artists");
    for (const a of artists ?? []) {
      const k = `artist:${a.id}`;
      const expanded = !!open[k];
      nodes.push({ key: k, level: 2, label: a.title, parent: "artists", expanded, on: route.name === "artist" && route.id === a.id, activate: go({ name: "artist", id: a.id }), toggle: () => openArtist(a.id, !expanded) });
      if (!expanded) continue;
      const list = albums[a.id];
      if (!list) loading(k);
      for (const al of list ?? [])
        nodes.push({ key: `album:${al.id}:${a.id}`, level: 3, label: al.year ? `${al.title} (${al.year})` : al.title, parent: k, on: route.name === "album" && route.id === al.id, activate: go({ name: "album", id: al.id }) });
    }
  }
  const crateGroup = (key: string, label: string, icon: ReactNode, list: Crate[] | null) => {
    // Only when the library has that data.
    if (!list?.length) return;
    if (!group(key, label, icon, list.length)) return;
    for (const c of list)
      nodes.push({ key: `${key}:${c.key}`, level: 2, label: c.name, count: c.count, parent: key, on: route.name === "crate" && route.kind === c.kind && route.key === c.key, activate: go({ name: "crate", kind: c.kind, key: c.key }) });
  };
  crateGroup("years", "Albums by year", <CalendarDays size={15} />, years);
  crateGroup("labels", "Labels", <Tag size={15} />, labels);

  const playlists = toEntries(items ?? [], owner).filter((e) => e.kind === "playlist");
  if (group("playlists", "Playlists", <ListMusic size={15} />, items ? playlists.length : undefined)) {
    if (!items) loading("playlists");
    for (const p of playlists) nodes.push({ key: `playlist:${p.id}`, level: 2, label: p.title, parent: "playlists", on: route.name === "playlist" && route.id === p.id, activate: () => openEntry(p) });
  }
  if (group("filters", "Filters", <SlidersHorizontal size={15} />)) {
    for (const f of Object.keys(FILTER_NAME) as ConsoleFilter[]) {
      const checked = filters.includes(f);
      nodes.push({
        key: `filter:${f}`,
        level: 2,
        label: FILTER_NAME[f],
        parent: "filters",
        checked,
        activate: () => useConsole.setState((s) => ({ filters: checked ? s.filters.filter((x) => x !== f) : [...s.filters, f] })),
      });
    }
  }
  return nodes;
}

/** The tree pane: search, then the tree, then Settings. */
export function ConsoleTree() {
  const nodes = useTreeNodes();
  const route = useCurrentEntry().route;
  const [focusKey, setFocusKey] = useState<string | null>(null);
  const list = useRef<HTMLUListElement>(null);
  const virtual = useRef<VirtualRowsHandle>(null);
  const pending = useRef<string | null>(null);
  const stop = focusKey && nodes.some((n) => n.key === focusKey) ? focusKey : nodes[0]?.key;

  useLayoutEffect(() => {
    const key = pending.current;
    if (!key) return;
    const el = list.current?.querySelector<HTMLElement>(`[data-key="${CSS.escape(key)}"]`);
    if (!el) return;
    pending.current = null;
    el.focus();
  });

  const focus = (i: number) => {
    const n = nodes[Math.max(0, Math.min(nodes.length - 1, i))];
    if (!n) return;
    setFocusKey(n.key);
    pending.current = n.key;
    virtual.current?.scrollToIndex(nodes.indexOf(n));
  };
  const onKey = (e: KeyboardEvent<HTMLUListElement>) => {
    const at = nodes.findIndex((n) => n.key === stop);
    const n = nodes[at];
    if (!n) return;
    switch (e.key) {
      case "ArrowDown":
        focus(at + 1);
        break;
      case "ArrowUp":
        focus(at - 1);
        break;
      case "Home":
        focus(0);
        break;
      case "End":
        focus(nodes.length - 1);
        break;
      case "ArrowRight":
        if (n.toggle && !n.expanded) n.toggle();
        else if (n.expanded) focus(at + 1);
        break;
      case "ArrowLeft":
        if (n.toggle && n.expanded) n.toggle();
        else if (n.parent) focus(nodes.findIndex((x) => x.key === n.parent));
        break;
      case "Enter":
      case " ":
        n.activate();
        break;
      default:
        return;
    }
    e.preventDefault();
  };

  return (
    <nav className="console-tree" aria-label="Main" data-testid="console-tree">
      <div className="console-tree__search">
        <SearchBox />
      </div>
      <div className="console-tree__scroll">
        <ul ref={list} className="console-tree__list" role="tree" aria-label="Library" onKeyDown={onKey}>
          <VirtualRows
            count={nodes.length}
            rowHeight={30}
            scroller=".console-tree__scroll"
            handle={virtual}
            renderRow={(i) => {
              const n = nodes[i];
              const expandable = n.expanded !== undefined;
              return (
                <li
                  key={n.key}
                  data-key={n.key}
                  role="treeitem"
                  aria-level={n.level}
                  aria-expanded={expandable ? n.expanded : undefined}
                  aria-selected={n.checked === undefined ? !!n.on : undefined}
                  aria-checked={n.checked}
                  aria-busy={n.key.endsWith(":loading") || undefined}
                  tabIndex={n.key === stop ? 0 : -1}
                  className={`console-node console-node--l${n.level} ${n.on ? "is-on" : ""} ${n.level === 1 && expandable ? "is-group" : ""}`}
                  onClick={() => {
                    setFocusKey(n.key);
                    n.activate();
                  }}
                  onFocus={() => setFocusKey(n.key)}
                >
                  <span
                    className="console-node__twist"
                    aria-hidden="true"
                    onClick={(e) => {
                      if (!n.toggle) return;
                      e.stopPropagation();
                      n.toggle();
                    }}
                  >
                    {expandable && (n.expanded ? <ChevronDown size={13} /> : <ChevronRight size={13} />)}
                  </span>
                  {n.checked !== undefined ? <span className={`console-node__check ${n.checked ? "is-on" : ""}`} aria-hidden="true" /> : n.icon && <span className="console-node__icon" aria-hidden="true">{n.icon}</span>}
                  <span className="console-node__label truncate">{n.key.endsWith(":loading") ? <Skeleton width="60%" height={10} /> : n.label}</span>
                  {n.count !== undefined && <span className="console-node__count tabular">{n.count.toLocaleString()}</span>}
                </li>
              );
            }}
          />
        </ul>
      </div>
      <div className="console-tree__foot">
        <button type="button" className={`console-foot-link ${route.name === "settings" ? "is-on" : ""}`} aria-current={route.name === "settings" ? "page" : undefined} onClick={() => nav.go({ name: "settings" })}>
          <SettingsIcon size={15} aria-hidden="true" />
          Settings
        </button>
        <StatusDot />
      </div>
    </nav>
  );
}

// ---------------------------------------------------------------- pane handles

const KEY_STEP = 16;

/**
 * A handle between panes: drag it, or focus it and use the arrow keys (Home
 * and End for the narrowest and widest). The width is kept on this device.
 */
export function PaneHandle({ pane }: { pane: Pane }) {
  const widths = usePaneWidths();
  const value = widths[pane];
  const l = PANE_LIMITS[pane];
  // The tree grows to the right; the details pane grows to the left.
  const sign = pane === "tree" ? 1 : -1;
  const drag = useRef<{ x: number; from: number } | null>(null);
  const field = pane === "tree" ? "consoleTree" : "consoleDetails";
  const commit = (px: number) => {
    useConsole.setState((s) => ({ live: { ...s.live, [pane]: undefined } }));
    void updateSettings({ [field]: clampPane(pane, px) });
  };
  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const next =
      e.key === "ArrowRight" ? value + sign * KEY_STEP : e.key === "ArrowLeft" ? value - sign * KEY_STEP : e.key === "Home" ? l.min : e.key === "End" ? l.max : null;
    if (next === null) return;
    e.preventDefault();
    commit(next);
  };
  const onDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    e.preventDefault();
    e.currentTarget.setPointerCapture?.(e.pointerId);
    drag.current = { x: e.clientX, from: value };
  };
  const at = (e: PointerEvent<HTMLDivElement>) => clampPane(pane, drag.current!.from + sign * (e.clientX - drag.current!.x));
  const onMove = (e: PointerEvent<HTMLDivElement>) => {
    if (!drag.current) return;
    const px = at(e);
    useConsole.setState((s) => ({ live: { ...s.live, [pane]: px } }));
  };
  const onUp = (e: PointerEvent<HTMLDivElement>) => {
    if (!drag.current) return;
    const px = at(e);
    drag.current = null;
    commit(px);
  };
  return (
    <div
      className={`console-handle console-handle--${pane}`}
      role="separator"
      aria-orientation="vertical"
      aria-label={pane === "tree" ? "Resize the tree" : "Resize the details pane"}
      aria-valuemin={l.min}
      aria-valuemax={l.max}
      aria-valuenow={value}
      tabIndex={0}
      onKeyDown={onKey}
      onPointerDown={onDown}
      onPointerMove={onMove}
      onPointerUp={onUp}
      onPointerCancel={onUp}
      onDoubleClick={() => commit(l.start)}
    />
  );
}

// ---------------------------------------------------------------- the details pane

function Facts({ rows }: { rows: [string, ReactNode][] }) {
  return (
    <dl className="console-facts">
      {rows.map(([k, v]) => (
        <div key={k} className="console-facts__row">
          <dt>{k}</dt>
          <dd className="tabular">{v}</dd>
        </div>
      ))}
    </dl>
  );
}

function Signal({ track, playing }: { track: Track; playing: boolean }) {
  const path = useSignal((s) => s.path);
  const stream = useStreamLabel();
  const current = playing && path?.trackId === track.id ? path : null;
  if (!current) {
    const q = qualityLabel(track);
    return (
      <>
        <Facts rows={[["Source", q ?? "Unknown format"]]} />
        <p className="console-note">{playing ? "Reading the path." : "The whole path shows while this song plays."}</p>
      </>
    );
  }
  return <Facts rows={stages(current, stream).map((s) => [s.name, s.detail ? `${s.value}, ${s.detail.charAt(0).toLowerCase()}${s.detail.slice(1)}` : s.value])} />;
}

function Credits({ track }: { track: Track }) {
  const { rows, loading } = useCreditRows(track);
  if (!rows.length) return <p className="console-note">{loading ? "Looking up credits." : "No credits for this song yet."}</p>;
  return (
    <ul className="console-credits">
      {rows.map((r) => (
        <li key={r.name}>
          <span className="console-credits__name truncate">{r.id ? <TextLink onActivate={() => nav.go({ name: "artist", id: r.id! })}>{r.name}</TextLink> : r.name}</span>
          <span className="console-credits__roles">{r.roles.join(", ")}</span>
        </li>
      ))}
    </ul>
  );
}

function ago(seconds: number | null): string | null {
  if (!seconds) return null;
  return new Date(seconds * 1000).toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" });
}

/** The details pane: the picked song, else the one playing. */
export function ConsoleDetails({ drawer = false }: { drawer?: boolean }) {
  const selected = useConsole((s) => s.selected);
  const playing = usePlayer((s) => s.state?.track ?? null);
  const track = selected ?? playing;
  const isPlaying = !!track && track.id === playing?.id;
  // As a drawer, Escape closes it (after any menu or dialog inside has had it).
  useEffect(() => {
    if (!drawer) return;
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented) return;
      if (document.querySelector("[role='dialog'][aria-modal='true'], [role='menu']")) return;
      useConsole.setState({ detailsOpen: false });
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [drawer]);
  return (
    <aside className={`console-details ${drawer ? "is-drawer" : ""}`} aria-label="Details" data-testid="console-details">
      {drawer && (
        <div className="console-details__close">
          <IconButton label="Close details" icon={<X size={16} />} onClick={() => useConsole.setState({ detailsOpen: false })} />
        </div>
      )}
      {track ? (
        <div className="console-details__body" key={track.id}>
          <div className="console-details__cover">
            <Art path={track.thumb} fluid requestSize={320} />
          </div>
          <div className="console-details__song">
            <span className="console-details__which">{isPlaying ? "Playing" : "Selected"}</span>
            <h2 className="console-details__title">{track.title}</h2>
            <ArtistLinks track={track} className="console-details__artist" />
            <TextLink className="console-details__album" onActivate={() => nav.go({ name: "album", id: track.albumId })}>
              {track.albumTitle}
            </TextLink>
          </div>
          <section className="console-section" aria-label="Tags">
            <h3 className="console-section__title">Tags</h3>
            <Facts
              rows={
                [
                  ["Album artist", track.artistTitle],
                  track.trackArtist ? ["Artist", track.trackArtist] : null,
                  ["Track", track.discNumber > 1 ? `${track.discNumber}-${track.trackNumber}` : String(track.trackNumber)],
                  ["Length", formatDuration(track.durationMs)],
                  ["Format", qualityLabel(track) ?? "Unknown"],
                  track.bitrate ? ["Bitrate", `${track.bitrate} kbps`] : null,
                  track.container ? ["File", track.container.toUpperCase()] : null,
                  track.genres?.length ? ["Genre", track.genres.join(", ")] : null,
                  ["Plays", track.viewCount.toLocaleString()],
                  ago(track.lastViewedAt) ? ["Last played", ago(track.lastViewedAt)] : null,
                ].filter(Boolean) as [string, ReactNode][]
              }
            />
          </section>
          <section className="console-section" aria-label="Signal path">
            <h3 className="console-section__title">Signal path</h3>
            <Signal track={track} playing={isPlaying} />
          </section>
          <section className="console-section" aria-label="Credits">
            <h3 className="console-section__title">Credits</h3>
            <Credits track={track} />
          </section>
        </div>
      ) : (
        <p className="console-details__empty">Pick a song in the table to see its tags, signal path and credits.</p>
      )}
    </aside>
  );
}

// ---------------------------------------------------------------- the status line

/** The one-line transport along the bottom. Its label and controls match the player bar's. */
export function ConsoleStatus({ detailsDrawer }: { detailsDrawer: boolean }) {
  const state = usePlayer((s) => s.state);
  const track = state?.track;
  const expanded = useFullWindow((s) => s.open);
  const nowPlayingOpen = useUi((s) => s.nowPlayingOpen) && !expanded;
  const like = useLayout().playerButtons.includes("like");
  const detailsOpen = useConsole((s) => s.detailsOpen);
  const nowText = track ? `Now playing: ${track.title} by ${trackArtistLine(track)}` : null;
  return (
    <aside className="console-status" aria-label="Player" data-testid="console-status">
      <NowPlayingAnnouncer text={nowText} />
      <div className="console-status__now" role="region" aria-label={nowText ?? "Nothing playing"}>
        {track ? (
          <>
            <button
              type="button"
              className="console-status__art"
              aria-label="Now playing view"
              aria-pressed={nowPlayingOpen}
              onClick={(e) => e.detail < 2 && useUi.getState().toggleNowPlaying()}
              onDoubleClick={() => {
                useUi.getState().toggleNowPlaying();
                fullWindow.open();
              }}
            >
              <Art path={track.thumb} size={28} />
            </button>
            <span className="console-status__song truncate">
              <TextLink className="console-status__title" onActivate={() => nav.go({ name: "album", id: track.albumId })}>
                {track.title}
              </TextLink>
              <span className="console-status__by"> by </span>
              <ArtistLinks track={track} className="console-status__artist" />
            </span>
            {like && <SaveButton track={track} place="bar" />}
          </>
        ) : state === null ? (
          <Skeleton width={160} height={12} />
        ) : (
          <span className="console-status__idle">Nothing playing</span>
        )}
      </div>
      <div className="console-status__keys">
        <PlayControls />
      </div>
      <div className="console-status__line">
        <SeekBar strip />
      </div>
      <StripExtras />
      {detailsDrawer && (
        <IconButton label="Details" tooltip="Details" pressed={detailsOpen} icon={<PanelRight size={18} />} onClick={() => useConsole.setState({ detailsOpen: !detailsOpen })} />
      )}
      <SignalPanel />
    </aside>
  );
}

