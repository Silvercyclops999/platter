/**
 * The library's items in one of four views (list, compact list, compact
 * grid, grid), as the sidebar list, the collapsed rail or the wide-mode page.
 * Virtualized by rows, so a library of thousands of albums only renders
 * what's on screen.
 */
import { observeElementRect, useVirtualizer, type Virtualizer } from "@tanstack/react-virtual";
import { Pause, Pin as PinIcon, Play, Volume2 } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState, type DragEvent, type KeyboardEvent, type MouseEvent, type ReactNode } from "react";
import { Art, FluidArtSize } from "../components/Art";
import { LikedCover } from "../components/LikedCover";
import { openContextMenu } from "../components/ContextMenu";
import { Skeleton } from "../components/Skeleton";
import { Tooltip } from "../components/Tooltip";
import { addedLabel, playedLabel } from "../lib/relativeTime";
import { albumDrag, dropOnLiked, dropOnPlaylist, endSongDrag, libraryDropRule, playlistDrag, startSongDrag, useSongDrop } from "../stores/drag";
import { KIND_LABEL, subtitle, typeLabel, type LibEntry, type LibView } from "../stores/library";
import { usePlayer } from "../stores/player";
import { sameRoute, useCurrentEntry } from "../stores/router";
import { useSettings } from "../stores/settings";
import { useUi } from "../stores/ui";
import { entryMenu, movePlaylist, openEntry, playEntry, type LibraryModel } from "./useLibrary";

export type CollectionMode = "sidebar" | "rail" | "wide";

/** Row heights (px). */
export const LIST_ROW = 64;
export const LIST_ROW_NO_ART = 48;
export const COMPACT_ROW = 32;
export const WIDE_COMPACT_ROW = 40;
export const RAIL_TILE = 56;
/** List art size. */
export const LIST_ART = 48;
const RAIL_ART = 48;
const GRID_GAP = 4;
/** Sidebar grids: smallest cell before a column drops. */
const SIDEBAR_GRID_MIN = 120;
const SIDEBAR_COMPACT_MIN = 88;
/** Wide grids: fixed cell widths; the column count grows with the window. */
export const WIDE_GRID_CELL = 176;
export const WIDE_COMPACT_CELL = 132;
/** Title and subtitle under grid art. */
const GRID_TEXT = 44;

const isGrid = (v: LibView) => v === "grid" || v === "compactGrid";

/**
 * The scroller's size, ignoring a zero height (not laid out yet, or no
 * layout at all as in tests) so the first screen of rows still renders.
 */
function observeNonEmptyRect(instance: Virtualizer<HTMLElement, Element>, cb: (rect: { width: number; height: number }) => void) {
  return observeElementRect(instance, (rect) => cb(rect.height > 0 ? rect : (instance.options.initialRect ?? rect)));
}

export interface Geometry {
  cols: number;
  art: number;
  pad: number;
  rowHeight: number;
}

/** Columns, art and row height for a view at a content width. */
export function geometry(view: LibView, mode: CollectionMode, width: number, showArt = true): Geometry {
  if (mode === "rail") return { cols: 1, art: RAIL_ART, pad: 4, rowHeight: RAIL_TILE };
  if (view === "list") return { cols: 1, art: LIST_ART, pad: 8, rowHeight: showArt ? LIST_ROW : LIST_ROW_NO_ART };
  if (view === "compactList") return { cols: 1, art: 0, pad: 8, rowHeight: mode === "wide" ? WIDE_COMPACT_ROW : COMPACT_ROW };
  const compact = view === "compactGrid";
  const pad = compact ? 6 : mode === "wide" ? 12 : 8;
  const w = width > 0 ? width : mode === "wide" ? 1024 : 276;

  let cols: number;
  if (mode === "wide") {
    // Cards stay about one size; wider windows get more columns. The few
    // spare pixels are shared out so the row ends flush.
    const base = compact ? WIDE_COMPACT_CELL : WIDE_GRID_CELL;
    cols = Math.max(1, Math.floor((w + GRID_GAP) / (base + GRID_GAP)));
  } else {
    const min = compact ? SIDEBAR_COMPACT_MIN : SIDEBAR_GRID_MIN;
    cols = Math.max(2, Math.floor((w + GRID_GAP) / (min + GRID_GAP)));
  }
  const cellWidth = (w - GRID_GAP * (cols - 1)) / cols;
  const art = Math.floor(cellWidth - pad * 2);
  return { cols, art, pad, rowHeight: Math.round(art + pad * 2 + (compact ? 0 : GRID_TEXT)) + GRID_GAP };
}

function useContextKey(): { key: string | null; playing: boolean } {
  const key = usePlayer((s) => {
    const c = s.state?.queue.context;
    return c?.id ? `${c.kind}:${c.id}` : null;
  });
  const playing = usePlayer((s) => s.state?.status === "playing" || s.state?.status === "buffering" || s.state?.status === "loading");
  return { key, playing };
}

interface ItemProps {
  e: LibEntry;
  index: number;
  tabIndex: number;
  selected: boolean;
  /** This item is the play context (playing or paused). */
  isContext: boolean;
  playing: boolean;
  filtered: boolean;
  onMenu: (e: LibEntry, ev: MouseEvent<HTMLElement> | HTMLElement) => void;
}

/**
 * Drag and drop for a library item: albums and playlists drag all their
 * songs; your playlists and Liked Songs take songs dropped on them, and
 * everything else dims for the drag.
 */
function useItemDnd(e: LibEntry) {
  const songDrags = useSettings((s) => s.settings?.dragAndDrop !== false);
  const drop = useSongDrop(
    e.liked ? "liked" : `${e.kind}:${e.id}`,
    e.title,
    (songs) => libraryDropRule(e, songs),
    (songs) => (e.liked ? dropOnLiked(songs) : dropOnPlaylist(e.id, songs)),
  );
  const source = songDrags && !e.unavailable && !e.liked && (e.kind === "album" || e.kind === "playlist");
  return {
    className: `${drop.over ? "is-drop-target" : ""} ${drop.dimmed ? "is-drop-dim" : ""}`,
    props: {
      ...drop.handlers,
      ...(source
        ? {
            draggable: true,
            onDragStart: (ev: DragEvent) => {
              startSongDrag(ev, e.kind === "album" ? albumDrag(e.id, e.title) : playlistDrag(e.id, e.title));
            },
            onDragEnd: endSongDrag,
          }
        : null),
    },
  };
}

function PlayOverlay({ e, playing, className }: { e: LibEntry; playing: boolean; className: string }) {
  if (e.unavailable) return null;
  return (
    <button
      type="button"
      tabIndex={-1}
      className={className}
      aria-label={`${playing ? "Pause" : "Play"} ${e.title}`}
      onClick={(ev) => {
        ev.stopPropagation();
        void playEntry(e);
      }}
      onDoubleClick={(ev) => ev.stopPropagation()}
    >
      {playing ? <Pause size={18} fill="currentColor" aria-hidden="true" /> : <Play size={18} fill="currentColor" aria-hidden="true" />}
    </button>
  );
}

const pinGlyph = <PinIcon size={12} className="lib-pin" aria-label="Pinned" />;

/** Default and compact list rows (sidebar and wide). */
function Row({ e, index, tabIndex, selected, isContext, playing, filtered, onMenu, compact, wide, showArt }: ItemProps & { compact: boolean; wide: boolean; showArt: boolean }) {
  const sub = subtitle(e, filtered);
  const added = wide ? addedLabel(e.addedAt) : "";
  const played = wide ? playedLabel(e.playedAt) : "";
  const dnd = useItemDnd(e);
  return (
    <div
      className={`lib-row ${compact ? "lib-row--compact" : ""} ${wide ? "lib-row--wide" : ""} ${selected ? "is-selected" : ""} ${dnd.className}`}
      role="listitem"
      {...dnd.props}
      onContextMenu={(ev) => {
        ev.preventDefault();
        onMenu(e, ev);
      }}
      onDoubleClick={() => !e.unavailable && void playEntry(e)}
    >
      {!compact && showArt && (
        <span className="lib-row__art">
          {e.liked ? <LikedCover size={LIST_ART} /> : <Art path={e.thumb} size={LIST_ART} round={e.kind === "artist"} kind={e.kind} />}
          <PlayOverlay e={e} playing={isContext && playing} className="lib-row__play" />
        </span>
      )}
      <button
        type="button"
        className="lib-row__open"
        data-index={index}
        tabIndex={tabIndex}
        aria-current={selected ? "page" : undefined}
        onClick={() => !e.unavailable && openEntry(e)}
      >
        {compact ? (
          <span className="lib-row__line">
            {e.pinned && pinGlyph}
            <span className={`lib-row__title ${isContext ? "is-playing" : ""}`}>{e.title}</span>
            {!e.unavailable && <span className="lib-row__type"> · {typeLabel(e)}</span>}
          </span>
        ) : (
          <span className="lib-row__text">
            <span className={`lib-row__title ${isContext ? "is-playing" : ""}`}>{e.title}</span>
            {(sub || e.pinned) && (
              <span className="lib-row__sub">
                {e.pinned && pinGlyph}
                <span className="lib-row__subtext">{sub}</span>
              </span>
            )}
          </span>
        )}
        {isContext && (
          <span className="lib-row__now" role="img" aria-label="Now playing">
            <Volume2 size={16} aria-hidden="true" />
          </span>
        )}
      </button>
      {wide && (
        <>
          <span className="lib-row__added">
            {added && <span className="sr-only">Added </span>}
            {added}
          </span>
          <span className="lib-row__played">
            {played && <span className="sr-only">Played </span>}
            {played}
          </span>
        </>
      )}
    </div>
  );
}

/** Grid and compact grid cells. */
function Cell({ e, index, tabIndex, selected, isContext, playing, filtered, onMenu, compact }: ItemProps & { compact: boolean }) {
  const sub = subtitle(e, filtered);
  const dnd = useItemDnd(e);
  return (
    <div
      className={`lib-cell ${compact ? "lib-cell--compact" : ""} ${selected ? "is-selected" : ""} ${dnd.className}`}
      role="listitem"
      {...dnd.props}
      onContextMenu={(ev) => {
        ev.preventDefault();
        onMenu(e, ev);
      }}
      onDoubleClick={() => !e.unavailable && void playEntry(e)}
    >
      <span className="lib-cell__art">
        {e.liked ? <LikedCover fluid /> : <Art path={e.thumb} fluid round={e.kind === "artist"} kind={e.kind} />}
        <PlayOverlay e={e} playing={isContext && playing} className={`lib-cell__play ${isContext && playing ? "is-on" : ""}`} />
      </span>
      <button
        type="button"
        className="lib-cell__open"
        data-index={index}
        tabIndex={tabIndex}
        aria-current={selected ? "page" : undefined}
        aria-label={compact ? [e.title, sub].filter(Boolean).join(", ") : undefined}
        onClick={() => !e.unavailable && openEntry(e)}
      >
        {!compact && (
          <>
            <span className={`lib-cell__title ${isContext ? "is-playing" : ""}`}>{e.title}</span>
            <span className="lib-cell__sub">
              {e.pinned && pinGlyph}
              <span className="lib-cell__subtext">{sub || " "}</span>
            </span>
          </>
        )}
      </button>
    </div>
  );
}

/** A rail tile: art only, with the title and subtitle in a tooltip. */
function Tile({ e, index, tabIndex, selected, onMenu }: ItemProps) {
  const sub = subtitle(e, false);
  const dnd = useItemDnd(e);
  return (
    <div className={`lib-tile-wrap ${dnd.className}`} role="listitem" onDoubleClick={() => !e.unavailable && void playEntry(e)} {...dnd.props}>
      <Tooltip
        placement="right"
        label={
          <span className="lib-tip">
            <span className="lib-tip__title">{e.title}</span>
            {sub && <span className="lib-tip__sub">{sub}</span>}
          </span>
        }
      >
        <button
          type="button"
          className={`lib-tile ${selected ? "is-selected" : ""}`}
          data-index={index}
          tabIndex={tabIndex}
          aria-label={[e.title, sub].filter(Boolean).join(", ")}
          aria-current={selected ? "page" : undefined}
          onClick={() => !e.unavailable && openEntry(e)}
          onContextMenu={(ev) => {
            ev.preventDefault();
            onMenu(e, ev);
          }}
        >
          {e.liked ? <LikedCover size={RAIL_ART} /> : <Art path={e.thumb} size={RAIL_ART} round={e.kind === "artist"} kind={e.kind} />}
        </button>
      </Tooltip>
    </div>
  );
}

function Empty({ model, query, mode }: { model: LibraryModel; query: string; mode: CollectionMode }) {
  if (mode === "rail") return null;
  const q = query.trim();
  if (q)
    return (
      <div className="lib-empty lib-empty--search" role="status">
        <p className="lib-empty__title">Couldn't find "{q}"</p>
        <p className="lib-empty__text">Check the spelling, or try fewer or different words.</p>
      </div>
    );
  return (
    <p className="lib-empty" role="status">
      {model.all.length === 0
        ? "Albums, artists and playlists from your Plex library show up here after the first sync."
        : model.filter === "all"
          ? "Nothing here yet."
          : `No ${model.sub === "smart" ? "smart playlists" : model.sub === "byYou" ? "playlists by you" : KIND_LABEL[model.filter].toLowerCase()} yet.`}
    </p>
  );
}

function Skeletons({ mode, view }: { mode: CollectionMode; view: LibView }) {
  if (mode === "wide" && isGrid(view))
    return (
      <div className="lib-skeleton-grid" aria-hidden="true">
        {Array.from({ length: 8 }, (_, i) => (
          <Skeleton key={i} width={view === "grid" ? WIDE_GRID_CELL - 24 : WIDE_COMPACT_CELL - 12} height={view === "grid" ? WIDE_GRID_CELL - 24 : WIDE_COMPACT_CELL - 12} radius="card" />
        ))}
      </div>
    );
  return (
    <div aria-hidden="true">
      {Array.from({ length: 6 }, (_, i) => (
        <div key={i} className={`lib-skeleton ${mode === "rail" ? "lib-skeleton--rail" : ""}`}>
          <Skeleton width={mode === "rail" ? RAIL_ART : LIST_ART} height={mode === "rail" ? RAIL_ART : LIST_ART} />
          {mode !== "rail" && (
            <span className="lib-skeleton__text">
              <Skeleton width="70%" height={13} />
              <Skeleton width="45%" height={11} style={{ marginTop: 6 }} />
            </span>
          )}
        </div>
      ))}
    </div>
  );
}

export interface LibraryCollectionProps {
  model: LibraryModel;
  view: LibView;
  mode: CollectionMode;
  /** Search text (for the no-matches message). */
  query: string;
  /** List rows show art (Settings). */
  showArt?: boolean;
  label?: string;
  /** Rendered above the rows inside the scroller (wide-mode table header). */
  header?: ReactNode;
}

export function LibraryCollection({ model, view: savedView, mode, query, showArt = true, label = "Your Library items", header }: LibraryCollectionProps) {
  const view: LibView = mode === "rail" ? "list" : savedView;
  const grid = mode !== "rail" && isGrid(view);
  const compact = view === "compactList" || view === "compactGrid";
  const own = mode !== "wide";
  const scrollRef = useRef<HTMLDivElement>(null);
  const root = useRef<HTMLDivElement>(null);
  const [scrollEl, setScrollEl] = useState<HTMLElement | null>(null);
  const [width, setWidth] = useState(0);
  const [offset, setOffset] = useState(0);
  const route = useCurrentEntry().route;
  const ctx = useContextKey();
  const entries = model.shown;
  const n = entries.length;
  const filtered = model.filter !== "all";

  useLayoutEffect(() => {
    const el = root.current;
    if (!el) return;
    const sc = own ? scrollRef.current : (el.closest<HTMLElement>(".main__scroll") ?? (document.scrollingElement as HTMLElement | null));
    setScrollEl(sc);
    const measure = () => {
      setWidth(el.clientWidth);
      if (!own && sc) setOffset(el.getBoundingClientRect().top - sc.getBoundingClientRect().top + sc.scrollTop);
    };
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [own, model.loading]);

  const g = geometry(view, mode, width, showArt);
  const rows = Math.ceil(n / g.cols);
  const v = useVirtualizer({
    count: rows,
    getScrollElement: () => scrollEl,
    estimateSize: () => g.rowHeight,
    overscan: 3,
    scrollMargin: own ? 0 : offset,
    initialRect: { width: own ? 300 : 1024, height: 800 },
    observeElementRect: observeNonEmptyRect,
  });
  useEffect(() => {
    v.measure();
  }, [g.rowHeight, g.cols, v]);

  // Keyboard focus: one tab stop in the list, arrows move it.
  const selectedIndex = entries.findIndex((e) => !e.unavailable && (e.liked ? route.name === "liked" : sameRoute(route, { name: e.kind, id: e.id })));
  const [cursor, setCursor] = useState<number | null>(null);
  const pending = useRef<number | null>(null);
  const vrows = v.getVirtualItems();
  const firstShown = (vrows[0]?.index ?? 0) * g.cols;
  const lastShown = Math.min(n, ((vrows[vrows.length - 1]?.index ?? -1) + 1) * g.cols) - 1;
  const want = Math.min(cursor ?? (selectedIndex >= 0 ? selectedIndex : 0), n - 1);
  const tabStop = want >= firstShown && want <= lastShown ? want : firstShown;

  useEffect(() => {
    const i = pending.current;
    if (i === null) return;
    const el = root.current?.querySelector<HTMLElement>(`[data-index="${i}"]`);
    if (el) {
      el.focus();
      pending.current = null;
    }
  });

  const focusIndex = (i: number) => {
    setCursor(i);
    pending.current = i;
    v.scrollToIndex(Math.floor(i / g.cols));
  };

  const onMenu = (e: LibEntry, at: MouseEvent<HTMLElement> | HTMLElement) => {
    const custom = model.sort === "custom" && e.kind === "playlist" && !e.pinned;
    let move;
    if (custom) {
      const list = entries.filter((x) => x.kind === "playlist" && !x.pinned);
      const i = list.findIndex((x) => x.id === e.id);
      move = { up: i > 0, down: i < list.length - 1, run: (by: -1 | 1) => movePlaylist(model.all, entries, e.id, by) };
    }
    const items = entryMenu(e, { playing: ctx.key === (e.liked ? "liked:liked" : `${e.kind}:${e.id}`) && ctx.playing, move });
    openContextMenu(`Options for ${e.title || "item"}`, items, at instanceof HTMLElement ? at : at.nativeEvent);
  };

  const onKeyDown = (ev: KeyboardEvent<HTMLDivElement>) => {
    const t = ev.target as HTMLElement;
    if (t.dataset.index === undefined) return;
    const i = Number(t.dataset.index);
    if (ev.key === "ContextMenu" || (ev.key === "F10" && ev.shiftKey)) {
      ev.preventDefault();
      onMenu(entries[i], t);
      return;
    }
    const step = { ArrowDown: g.cols, ArrowUp: -g.cols, ArrowRight: grid ? 1 : 0, ArrowLeft: grid ? -1 : 0 }[ev.key];
    if (!step) return;
    ev.preventDefault();
    const next = i + step;
    if (next >= 0 && next < n) focusIndex(next);
  };

  const item = (e: LibEntry, i: number) => {
    // Liked Songs plays as its own context kind.
    const key = e.liked ? "liked:liked" : `${e.kind}:${e.id}`;
    const p: ItemProps = {
      e,
      index: i,
      tabIndex: i === tabStop ? 0 : -1,
      selected: i === selectedIndex,
      isContext: ctx.key === key && !e.unavailable,
      playing: ctx.playing,
      filtered,
      onMenu,
    };
    if (mode === "rail") return <Tile key={key} {...p} />;
    if (grid) return <Cell key={key} {...p} compact={compact} />;
    return <Row key={key} {...p} compact={compact} wide={mode === "wide"} showArt={showArt} />;
  };

  let body: ReactNode;
  if (model.loading) body = <Skeletons mode={mode} view={view} />;
  else if (n === 0) body = <Empty model={model} query={query} mode={mode} />;
  else
    body = (
      <FluidArtSize.Provider value={grid ? g.art : null}>
        <div
          role="list"
          aria-label={label}
          className={`lib-items lib-items--${mode} lib-items--${view} ${compact ? "is-compact" : ""}`}
          style={{ height: v.getTotalSize(), position: "relative", ["--lib-pad" as string]: `${g.pad}px` }}
          onKeyDown={onKeyDown}
        >
          {vrows.map((r) => (
            <div
              key={r.key}
              className="lib-vrow"
              style={{
                position: "absolute",
                top: 0,
                left: 0,
                width: "100%",
                height: grid ? g.rowHeight - GRID_GAP : g.rowHeight,
                transform: `translateY(${r.start - v.options.scrollMargin}px)`,
                ...(grid
                  ? {
                      display: "grid",
                      gridTemplateColumns: `repeat(${g.cols}, minmax(0, 1fr))`,
                      columnGap: GRID_GAP,
                    }
                  : null),
              }}
            >
              {Array.from({ length: g.cols }, (_, c) => {
                const i = r.index * g.cols + c;
                return i < n ? item(entries[i], i) : null;
              })}
            </div>
          ))}
        </div>
      </FluidArtSize.Provider>
    );

  const inner = (
    <>
      {header}
      <div ref={root} className="lib-collection">
        {body}
      </div>
    </>
  );
  if (!own) return inner;
  return (
    <div ref={scrollRef} className={`lib-scroll lib-scroll--${mode}`}>
      {inner}
    </div>
  );
}

/** Wide-mode list header: title, date added, played. */
export function LibraryTableHead({ compact }: { compact: boolean }) {
  const showArt = useUi((s) => s.sidebarArt);
  return (
    <div className={`lib-thead ${compact ? "lib-thead--compact" : ""} ${!compact && showArt ? "lib-thead--art" : ""}`} aria-hidden="true">
      <span>Title</span>
      <span>Date added</span>
      <span className="lib-thead__played">Played</span>
    </div>
  );
}
