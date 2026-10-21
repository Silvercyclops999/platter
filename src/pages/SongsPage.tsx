/**
 * Your Library > Songs: every song in the library as a table. Click a
 * header to sort by it, again to flip; right-click a header to show or hide
 * columns. The core sends the rows a page at a time as they scroll into
 * view, so a very large library scrolls smoothly. A line at the foot counts
 * what's in view, or what's selected.
 */
import { observeElementRect, useVirtualizer, type Virtualizer } from "@tanstack/react-virtual";
import { ArrowDown, ArrowUp, Pause, Play, Search } from "lucide-react";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent, type MouseEvent } from "react";
import { create } from "zustand";
import { Art } from "../components/Art";
import { IconButton } from "../components/Button";
import { openContextMenu } from "../components/ContextMenu";
import { ExplicitMark } from "../components/FileTagBadges";
import { LoadError } from "../components/LoadError";
import { Skeleton } from "../components/Skeleton";
import { trackMenuItems } from "../components/TrackRow";
import { backend } from "../lib/backend";
import { formatDuration, formatLongDuration, plural, qualityLabel, trackArtistLine } from "../lib/format";
import type { SongRow, SongSort, SongsQuery } from "../lib/types";
import { endSongDrag, startSongDrag, tracksDrag } from "../stores/drag";
import { player, usePlayer } from "../stores/player";
import { useSettings } from "../stores/settings";
import { usePageBar } from "./detail";
import "./songs.css";

/** Columns after the title, which always shows (with the artist under it). */
export type SongColumn = "album" | "year" | "time" | "plays" | "added" | "format" | "lastPlayed" | "rating" | "trackNumber";
export const SONG_COLUMNS: readonly SongColumn[] = ["trackNumber", "album", "year", "format", "rating", "plays", "lastPlayed", "added", "time"];
export const DEFAULT_COLUMNS: SongColumn[] = ["album", "year", "plays", "added", "time"];
export const COLUMN_LABEL: Record<SongColumn | "title", string> = {
  title: "Title",
  album: "Album",
  year: "Year",
  time: "Time",
  plays: "Plays",
  added: "Added",
  format: "Format",
  lastPlayed: "Last played",
  rating: "Rating",
  trackNumber: "Track",
};
const WIDTH: Record<SongColumn, string> = {
  album: "minmax(120px, 2fr)",
  year: "56px",
  time: "64px",
  plays: "64px",
  added: "104px",
  format: "minmax(84px, 0.8fr)",
  lastPlayed: "104px",
  rating: "72px",
  trackNumber: "52px",
};
const NUMERIC: readonly SongColumn[] = ["year", "time", "plays", "trackNumber"];
/** A newly sorted column starts with the most first for these. */
const BIGGEST_FIRST: readonly (SongSort | "title")[] = ["plays", "added", "lastPlayed", "rating", "format"];

export const PAGE = 200;
const ROW_H = 52;

// ---------------------------------------------------------------- what's kept on this device

interface TablePrefs {
  columns: SongColumn[];
  sort: SongSort;
  desc: boolean;
}
const KEY = "platter.songsTable.v1"; // gitleaks:allow (localStorage key name, not a secret)
const SORTS: readonly SongSort[] = ["title", "artist", "album", "year", "time", "plays", "added", "lastPlayed", "rating", "trackNumber", "format"];

export function loadTablePrefs(raw: string | null): TablePrefs {
  const fallback: TablePrefs = { columns: DEFAULT_COLUMNS, sort: "title", desc: false };
  try {
    const p = JSON.parse(raw ?? "null") as Partial<TablePrefs> | null;
    if (!p) return fallback;
    const columns = Array.isArray(p.columns) ? SONG_COLUMNS.filter((c) => p.columns!.includes(c)) : fallback.columns;
    return { columns, sort: SORTS.includes(p.sort as SongSort) ? (p.sort as SongSort) : "title", desc: p.desc === true };
  } catch {
    return fallback;
  }
}

export const useSongsTable = create<TablePrefs>()(() => {
  try {
    return loadTablePrefs(localStorage.getItem(KEY));
  } catch {
    return loadTablePrefs(null);
  }
});

function setPrefs(p: Partial<TablePrefs>) {
  useSongsTable.setState(p);
  try {
    localStorage.setItem(KEY, JSON.stringify(useSongsTable.getState()));
  } catch {
    // Storage refused: the choice holds until the window closes.
  }
}

/** A header click: sort by it, or flip it when it's the sort already. */
export function clickSort(cur: { sort: SongSort; desc: boolean }, key: SongSort): { sort: SongSort; desc: boolean } {
  if (cur.sort === key) return { sort: key, desc: !cur.desc };
  return { sort: key, desc: BIGGEST_FIRST.includes(key) };
}

/** "12,408 songs, 34 days 2 hr" */
export function totalsLine(n: number, ms: number, selected = false): string {
  const days = Math.floor(ms / 86_400_000);
  const time = days > 0 ? `${plural(days, "day")} ${Math.floor((ms % 86_400_000) / 3_600_000)} hr` : formatLongDuration(ms);
  return `${plural(n, "song")}${selected ? " selected" : ""}, ${time}`;
}

// ---------------------------------------------------------------- the rows, a page at a time

interface Rows {
  total: number | null;
  totalMs: number;
  pages: Map<number, SongRow[]>;
  error: unknown;
}

/**
 * The table's rows for `q`: pages load as `want` asks for them. A new order
 * or find starts over; the library changing (a play count, a sync) asks
 * again but keeps the rows on screen until the new ones arrive.
 */
function useSongRows(q: SongsQuery) {
  const [rows, setRows] = useState<Rows>({ total: null, totalMs: 0, pages: new Map(), error: null });
  const asked = useRef(new Set<number>());
  const gen = useRef(0);
  const [version, setVersion] = useState(0);
  const shownFor = useRef("");

  const want = useCallback(
    (page: number) => {
      if (asked.current.has(page)) return;
      asked.current.add(page);
      const g = gen.current;
      backend.songsPage(q, page * PAGE, PAGE).then(
        (p) => {
          if (g !== gen.current) return;
          setRows((r) => ({ total: p.total ?? r.total, totalMs: p.totalMs ?? r.totalMs, pages: new Map(r.pages).set(page, p.rows), error: null }));
        },
        (e) => {
          if (g !== gen.current) return;
          // It stays asked, so it isn't asked again and again; Try again starts over.
          setRows((r) => ({ ...r, error: e }));
        },
      );
    },
    // A new query is a new loader.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [q.sort, q.desc, q.find, version],
  );

  useLayoutEffect(() => {
    gen.current++;
    asked.current = new Set();
    const key = JSON.stringify([q.sort, q.desc, q.find]);
    if (key !== shownFor.current) {
      shownFor.current = key;
      setRows((r) => ({ total: r.total, totalMs: r.totalMs, pages: new Map(), error: null }));
    } else setRows((r) => ({ ...r, error: null }));
    want(0);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [want]);

  // New songs, a sync or a changed view: start over where you are.
  useEffect(() => {
    let off: (() => void) | undefined;
    let gone = false;
    void backend.listen("library://changed", () => setVersion((v) => v + 1)).then((u) => (gone ? u() : (off = u)));
    return () => {
      gone = true;
      off?.();
    };
  }, []);

  const retry = useCallback(() => setVersion((v) => v + 1), []);
  return { ...rows, want, retry };
}

// ---------------------------------------------------------------- cells

const dateFmt = new Intl.DateTimeFormat(undefined, { day: "numeric", month: "short", year: "numeric" });
const day = (s: number | null | undefined) => (s ? dateFmt.format(new Date(s * 1000)) : "");

function cellText(r: SongRow, c: SongColumn): string {
  const t = r.track;
  switch (c) {
    case "album":
      return t.albumTitle;
    case "year":
      return r.year ? String(r.year) : "";
    case "time":
      return formatDuration(t.durationMs);
    case "plays":
      return t.viewCount ? t.viewCount.toLocaleString() : "";
    case "added":
      return day(r.addedAt);
    case "format":
      return qualityLabel(t) ?? (t.codec ? t.codec.toUpperCase() : "");
    case "lastPlayed":
      return day(t.lastViewedAt);
    case "rating":
      // Plex rates out of 10; five stars show it.
      return t.userRating ? "★".repeat(Math.round(t.userRating / 2)) : "";
    case "trackNumber":
      return t.trackNumber ? String(t.trackNumber) : "";
  }
}

// ---------------------------------------------------------------- the page

function observeNonEmptyRect(instance: Virtualizer<HTMLElement, Element>, cb: (rect: { width: number; height: number }) => void) {
  return observeElementRect(instance, (rect) => cb(rect.height > 0 ? rect : (instance.options.initialRect ?? rect)));
}

export function SongsPage() {
  const { columns, sort, desc } = useSongsTable();
  const [find, setFind] = useState("");
  const [found, setFound] = useState("");
  // Typing waits a moment before asking the core again.
  useEffect(() => {
    const t = setTimeout(() => setFound(find.trim()), 200);
    return () => clearTimeout(t);
  }, [find]);
  const q = useMemo<SongsQuery>(() => ({ sort, desc, find: found }), [sort, desc, found]);
  const { total, totalMs, pages, error, want, retry } = useSongRows(q);
  const nowId = usePlayer((s) => s.state?.track?.id);
  const status = usePlayer((s) => s.state?.status);
  const ctxId = usePlayer((s) => (s.state?.queue.context?.kind === "tracks" ? s.state.queue.context.id : null));
  const songDrags = useSettings((s) => s.settings?.dragAndDrop !== false);
  const isThis = ctxId === "songs";
  const playingHere = isThis && status === "playing";

  const rowAt = useCallback((i: number): SongRow | undefined => pages.get(Math.floor(i / PAGE))?.[i % PAGE], [pages]);
  const play = (start?: string) =>
    void backend
      .songsPlayIds(q, start)
      .then(({ trackIds }) => trackIds.length && player.playSongs(trackIds, start ?? trackIds[0]))
      .catch(() => undefined);
  const toggle = () => (isThis ? player.toggle() : play());
  usePageBar("Songs", playingHere, toggle);

  // Selection by song (so a refresh that moves rows keeps the same songs), with the row anchor for Shift ranges.
  const [selected, setSelected] = useState<ReadonlyMap<string, SongRow>>(() => new Map());
  const [focus, setFocus] = useState(0);
  const anchor = useRef(0);
  useEffect(() => {
    setSelected(new Map());
    setFocus(0);
  }, [q]);

  const root = useRef<HTMLDivElement>(null);
  const [rowsEl, setRowsEl] = useState<HTMLDivElement | null>(null);
  const [scrollEl, setScrollEl] = useState<HTMLElement | null>(null);
  const [offset, setOffset] = useState(0);
  // Rows are placed from where the first one starts (below the sticky header).
  useLayoutEffect(() => {
    const el = rowsEl;
    if (!el) return;
    const sc = el.closest<HTMLElement>(".main__scroll") ?? (document.scrollingElement as HTMLElement | null);
    setScrollEl(sc);
    const measure = () => sc && setOffset(el.getBoundingClientRect().top - sc.getBoundingClientRect().top + sc.scrollTop);
    measure();
    if (typeof ResizeObserver === "undefined" || !sc) return;
    const ro = new ResizeObserver(measure);
    ro.observe(sc);
    // A line appearing above the table ("Some songs didn't load") moves where the rows start.
    const content = sc.firstElementChild;
    if (content && content !== el) ro.observe(content);
    return () => ro.disconnect();
  }, [rowsEl]);

  const count = total ?? 0;
  const v = useVirtualizer({
    count,
    getScrollElement: () => scrollEl,
    estimateSize: () => ROW_H,
    overscan: 10,
    scrollMargin: offset,
    initialRect: { width: 1024, height: 900 },
    observeElementRect: observeNonEmptyRect,
  });
  const items = v.getVirtualItems();
  // Ask for the pages the rows on screen are in.
  useEffect(() => {
    for (const it of items) want(Math.floor(it.index / PAGE));
  });

  const template = ["minmax(200px, 3fr)", ...columns.map((c) => WIDTH[c])].join(" ");

  /** Loaded rows from a to b, by song. */
  const span = (a: number, b: number) => {
    const out = new Map<string, SongRow>();
    for (let k = a; k <= b; k++) {
      const r = rowAt(k);
      if (r) out.set(r.track.id, r);
    }
    return out;
  };
  const pick = (i: number, e: MouseEvent | KeyboardEvent) => {
    setFocus(i);
    const row = rowAt(i);
    if (e.shiftKey) {
      setSelected(span(Math.min(anchor.current, i), Math.max(anchor.current, i)));
      return;
    }
    anchor.current = i;
    if (!row) return;
    if (e.ctrlKey || e.metaKey) {
      setSelected((s) => {
        const n = new Map(s);
        if (n.has(row.track.id)) n.delete(row.track.id);
        else n.set(row.track.id, row);
        return n;
      });
    } else setSelected(new Map([[row.track.id, row]]));
  };
  const chosen = () => [...selected.values()];

  const moveTo = (i: number, e: KeyboardEvent) => {
    const to = Math.max(0, Math.min(count - 1, i));
    v.scrollToIndex(to, { align: "auto" });
    pick(to, e);
    requestAnimationFrame(() => root.current?.querySelector<HTMLElement>(`[data-row="${to}"]`)?.focus());
  };
  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const step: Record<string, number> = { ArrowDown: 1, ArrowUp: -1, PageDown: 10, PageUp: -10 };
    if (e.key in step) {
      e.preventDefault();
      moveTo(focus + step[e.key], e);
    } else if (e.key === "Home" || e.key === "End") {
      e.preventDefault();
      moveTo(e.key === "Home" ? 0 : count - 1, e);
    } else if (e.key === "Enter") {
      const r = rowAt(focus);
      if (r) {
        e.preventDefault();
        play(r.track.id);
      }
    } else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "a") {
      e.preventDefault();
      setSelected(span(0, count - 1));
    }
  };

  const columnsMenu = (e: MouseEvent) => {
    e.preventDefault();
    openContextMenu(
      "Columns",
      SONG_COLUMNS.map((c) => ({
        label: COLUMN_LABEL[c],
        icon: null,
        checked: columns.includes(c),
        onSelect: () => setPrefs({ columns: SONG_COLUMNS.filter((x) => (x === c ? !columns.includes(c) : columns.includes(x))) }),
      })),
      e,
    );
  };

  const head = (c: SongColumn | "title") => {
    const key: SongSort = c;
    const on = sort === key;
    return (
      <div key={c} role="columnheader" aria-sort={on ? (desc ? "descending" : "ascending") : "none"} className={`songs__cell songs__cell--${c}`} onContextMenu={columnsMenu}>
        <button type="button" className={`songs__sort ${on ? "is-on" : ""} ${c !== "title" && NUMERIC.includes(c) ? "is-numeric" : ""}`} onClick={() => setPrefs(clickSort({ sort, desc }, key))}>
          {COLUMN_LABEL[c]}
          <span className="songs__mark" aria-hidden="true">
            {on && (desc ? <ArrowDown size={12} /> : <ArrowUp size={12} />)}
          </span>
        </button>
      </div>
    );
  };

  const picked = chosen();
  const foot =
    total === null
      ? null
      : picked.length > 1
        ? totalsLine(picked.length, picked.reduce((n, r) => n + r.track.durationMs, 0), true)
        : totalsLine(total, totalMs);

  let body;
  if (error && total === null) {
    body = <LoadError what="your songs" error={error} onRetry={retry} inline />;
  } else if (total === 0) {
    body = <p className="songs__none">{found ? `No songs match "${found}"` : "No songs in your library yet."}</p>;
  } else {
    body = (
      <div ref={setRowsEl} style={{ height: total === null ? ROW_H * 8 : v.getTotalSize(), position: "relative" }} role="rowgroup">
        {(total === null ? Array.from({ length: 8 }, (_, i) => ({ index: i, start: i * ROW_H + offset, key: i })) : items).map((it) => {
          const r = total === null ? undefined : rowAt(it.index);
          const style = { position: "absolute" as const, top: 0, left: 0, width: "100%", height: ROW_H, transform: `translateY(${it.start - offset}px)`, gridTemplateColumns: template };
          if (!r) {
            // Still loading, but a row keys can land on (End, Page Down).
            const real = total !== null;
            return (
              <div
                key={it.key}
                role="row"
                data-row={real ? it.index : undefined}
                aria-rowindex={it.index + 2}
                aria-busy="true"
                tabIndex={real ? (focus === it.index ? 0 : -1) : undefined}
                onFocus={real ? () => setFocus(it.index) : undefined}
                className="songs__row songs__row--wait"
                style={style}
              >
                <div role="gridcell" className="songs__cell songs__cell--title">
                  <Skeleton width={32} height={32} />
                  <Skeleton width={`${40 + ((it.index * 17) % 40)}%`} height={11} />
                </div>
                {columns.map((c) => (
                  <div key={c} role="gridcell" className={`songs__cell songs__cell--${c}`} />
                ))}
              </div>
            );
          }
          const t = r.track;
          const on = selected.has(t.id);
          const now = t.id === nowId;
          return (
            <div
              key={it.key}
              role="row"
              data-row={it.index}
              data-track-id={t.id}
              aria-rowindex={it.index + 2}
              aria-selected={on}
              tabIndex={focus === it.index ? 0 : -1}
              className={`songs__row ${on ? "is-selected" : ""} ${now ? "is-playing" : ""}`}
              style={style}
              onClick={(e) => pick(it.index, e)}
              onFocus={() => setFocus(it.index)}
              onDoubleClick={() => play(t.id)}
              onContextMenu={(e) => {
                e.preventDefault();
                if (!selected.has(t.id)) pick(it.index, e);
                openContextMenu(`Options for ${t.title}`, trackMenuItems(t), e);
              }}
              draggable={songDrags}
              onDragStart={
                songDrags
                  ? (e) => {
                      const many = selected.has(t.id) ? chosen().map((x) => x.track) : [t];
                      void startSongDrag(e, tracksDrag(many));
                    }
                  : undefined
              }
              onDragEnd={songDrags ? endSongDrag : undefined}
            >
              <div role="gridcell" className="songs__cell songs__cell--title">
                <span className="songs__art">
                  <Art path={t.thumb} size={32} sleeve={{ title: t.albumTitle, artist: t.artistTitle }} />
                  {now && <span className={`songs__now ${status === "playing" ? "is-on" : ""}`} aria-label={status === "playing" ? "Playing" : "Paused"} />}
                </span>
                <span className="songs__text">
                  <span className="songs__title truncate">
                    {t.title}
                    <ExplicitMark trackId={t.id} />
                  </span>
                  <span className="songs__by truncate">{trackArtistLine(t)}</span>
                </span>
              </div>
              {columns.map((c) => (
                <div key={c} role="gridcell" className={`songs__cell songs__cell--${c} ${NUMERIC.includes(c) ? "tabular" : ""}`}>
                  <span className="truncate">{cellText(r, c)}</span>
                </div>
              ))}
            </div>
          );
        })}
      </div>
    );
  }

  return (
    <div className="page-pad songs">
      <header className="songs__head">
        <IconButton
          label={playingHere ? "Pause your songs" : "Play your songs"}
          variant="accent"
          size="lg"
          disabled={!total}
          icon={playingHere ? <Pause size={20} fill="currentColor" /> : <Play size={20} fill="currentColor" className="nudge-right" />}
          onClick={toggle}
        />
        <h1 className="songs__name">Songs</h1>
        <label className="songs__find">
          <Search size={16} aria-hidden="true" />
          <input type="search" placeholder="Find a song" aria-label="Find a song" value={find} spellCheck={false} autoComplete="off" onChange={(e) => setFind(e.target.value)} />
        </label>
      </header>
      {!!error && total !== null && (
        <p className="songs__error" role="alert">
          Some songs didn't load.{" "}
          <button type="button" className="songs__retry" onClick={retry}>
            Try again
          </button>
        </p>
      )}
      <div
        ref={root}
        className="songs__grid"
        role="grid"
        aria-label="Your songs"
        aria-rowcount={total === null ? -1 : total + 1}
        aria-colcount={columns.length + 1}
        aria-busy={total === null || undefined}
        onKeyDown={onKey}
      >
        <div role="row" aria-rowindex={1} className="songs__row songs__row--head" style={{ gridTemplateColumns: template }}>
          {head("title")}
          {columns.map(head)}
        </div>
        {body}
      </div>
      {foot && <p className="songs__foot tabular">{foot}</p>}
    </div>
  );
}
