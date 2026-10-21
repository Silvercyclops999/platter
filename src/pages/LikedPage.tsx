import { observeElementRect, useVirtualizer, type Virtualizer } from "@tanstack/react-virtual";
import { ArrowDownUp, Heart, List, Pause, Play, Rows3, Search, X } from "lucide-react";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type MutableRefObject } from "react";
import { Button, IconButton } from "../components/Button";
import { ContextShuffleButton } from "../components/ShuffleButton";
import { openContextMenu } from "../components/ContextMenu";
import { LikedCover } from "../components/LikedCover";
import { DownloadButton } from "../components/DownloadButton";
import { ChipRow } from "../components/Tabs";
import { Skeleton } from "../components/Skeleton";
import { TrackList, TrackRow, TrackRowSkeleton } from "../components/TrackRow";
import { backend } from "../lib/backend";
import { NEUTRAL_HEADER } from "../lib/color";
import { formatLongDuration, plural } from "../lib/format";
import { addedLabel } from "../lib/relativeTime";
import type { Track } from "../lib/types";
import { useLoad } from "../lib/useLoad";
import { useApp } from "../stores/app";
import { useLikes } from "../stores/likes";
import { player, usePlayer } from "../stores/player";
import { nav } from "../stores/router";
import { useSettings } from "../stores/settings";
import { useUi } from "../stores/ui";
import { useContextState } from "./cards";
import { usePageBar, useTitleStep } from "./detail";
import {
  LIKED_SORT_LABEL,
  LIKED_VIEW_LABEL,
  likedChips,
  likedRows,
  loadLikedPrefs,
  saveLikedPrefs,
  type LikedPrefs,
  type LikedSort,
  type LikedView,
} from "./liked";
import "./liked.css";
import { usePageFind, usePageReveal } from "../shell/pageFind";
import { FacetChips, FacetsEmpty, facetSongs, songFacts, useAlbumFacts, useFacets } from "../shell/FacetChips";
import { NO_CHIPS, chipOptions, chipsOn } from "../lib/libraryChips";
import { useLibraryView } from "../stores/library";
import { useLibraryLoader } from "../shell/useLibrary";
import { LoadError } from "../components/LoadError";

export const LIKED_ID = "liked";
const TITLE = "Liked Songs";
/** Header wash: warm ivory from the palette, to go with the cover. */
const LIKED_TINT = { vivid: "#6E6658", shade: "#24211D" };
const ROW_H: Record<LikedView, number> = { list: 56, compact: 40 };

/** The scroller's size, ignoring a zero height (no layout yet, or tests). */
function observeNonEmptyRect(instance: Virtualizer<HTMLElement, Element>, cb: (rect: { width: number; height: number }) => void) {
  return observeElementRect(instance, (rect) => cb(rect.height > 0 ? rect : (instance.options.initialRect ?? rect)));
}

/** Refetch the page whenever likes change (here, rolled back, or from Plex). */
function useLikesVersion(): number {
  const [v, setV] = useState(0);
  useEffect(() => {
    let un: (() => void) | null = null;
    let live = true;
    backend.listen("likes://changed", () => setV((n) => n + 1)).then((u) => (live ? (un = u) : u()));
    return () => {
      live = false;
      un?.();
    };
  }, []);
  return v;
}

export function LikedPage() {
  const version = useLikesVersion();
  const { data, error, retry } = useLoad(() => backend.likedPage(), [LIKED_ID, version]);
  const liked = useLikes((s) => s.ids);
  const owner = useApp((s) => s.auth?.username ?? null);
  const colours = useSettings((s) => s.settings?.artworkColours !== false);
  const nowTrackId = usePlayer((s) => s.state?.track?.id);
  const status = usePlayer((s) => s.state?.status);
  const { isContext, playing: isThisPlaying } = useContextState("liked", LIKED_ID);
  const [prefs, setPrefs] = useState<LikedPrefs>(loadLikedPrefs);
  const [chip, setChip] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [findOpen, setFindOpen] = useState(false);
  const findRef = useRef<HTMLInputElement>(null);

  useEffect(() => useUi.getState().setTint(colours ? LIKED_TINT : NEUTRAL_HEADER), [colours]);

  // Songs unliked while the page is open stay, dimmed, until you leave.
  const [ghosts, setGhosts] = useState<Map<string, Track>>(() => new Map());
  const shown = useRef<Map<string, Track>>(new Map());
  // Songs in the list when it first loaded; later ones fade in at the top.
  const known = useRef<Set<string> | null>(null);
  const [entering, setEntering] = useState<ReadonlySet<string>>(() => new Set());

  useEffect(() => {
    if (!data) return;
    for (const t of data.tracks) shown.current.set(t.id, t);
    if (!known.current) {
      known.current = new Set(data.tracks.map((t) => t.id));
      return;
    }
    const fresh = data.tracks.filter((t) => !known.current!.has(t.id)).map((t) => t.id);
    fresh.forEach((id) => known.current!.add(id));
    if (fresh.length) setEntering(new Set(fresh));
  }, [data]);

  useEffect(() => {
    setGhosts((g) => {
      const next = new Map(g);
      let changed = false;
      for (const [id, t] of shown.current) {
        if (!liked.has(id) && !next.has(id) && useLikes.getState().loaded) {
          next.set(id, t);
          changed = true;
        } else if (liked.has(id) && next.delete(id)) changed = true;
      }
      return changed ? next : g;
    });
  }, [liked]);

  const all = useMemo(() => {
    if (!data) return [];
    const have = new Set(data.tracks.map((t) => t.id));
    return [...data.tracks, ...[...ghosts.values()].filter((t) => !have.has(t.id))];
  }, [data, ghosts]);
  const chips = useMemo(() => likedChips(data?.tracks ?? []), [data]);
  const activeChip = chip && chips.some((c) => c.tag === chip) ? chip : null;
  // Filter chips (type, files, decade, added recently), from each song's album.
  useLibraryLoader();
  const libItems = useLibraryView((s) => s.items);
  const facts = useAlbumFacts(libItems);
  const [facets, setFacets] = useFacets("songs");
  const facetOpts = useMemo(() => chipOptions(all.map((t) => songFacts(t, facts.byAlbum)), Math.floor(Date.now() / 1000)), [all, facts]);
  const rows = useMemo(
    () => facetSongs(likedRows(all, { sort: prefs.sort, chip: activeChip, query }), facets, facts.byAlbum),
    [all, prefs.sort, activeChip, query, facets, facts],
  );
  const counted = activeChip ? rows.filter((t) => liked.has(t.id)) : (data?.tracks ?? []);
  const narrowed = prefs.sort !== "added" || !!activeChip || !!query.trim() || chipsOn(facets);
  const empty = !!data && data.tracks.length === 0 && ghosts.size === 0;

  const playFrom = (trackId?: string) => {
    const ids = rows.filter((t) => liked.has(t.id)).map((t) => t.id);
    return player.playLiked(trackId, narrowed ? ids : undefined);
  };
  const play = () => (isContext ? player.toggle() : playFrom());
  usePageBar(TITLE, isThisPlaying, play);
  const { header, column, step, compact } = useTitleStep(TITLE);

  const setPref = (p: Partial<LikedPrefs>) =>
    setPrefs((cur) => {
      const next = { ...cur, ...p };
      saveLikedPrefs(next);
      return next;
    });

  const sortMenu = (el: HTMLElement) =>
    openContextMenu(
      "Sort and view",
      [
        ...(["title", "added", "artist", "album"] as LikedSort[]).map((s, i) => ({
          heading: i === 0 ? "Sort by" : undefined,
          label: LIKED_SORT_LABEL[s],
          checked: prefs.sort === s,
          onSelect: () => setPref({ sort: s }),
        })),
        ...(["compact", "list"] as LikedView[]).map((v, i) => ({
          heading: i === 0 ? "View as" : undefined,
          separator: i === 0,
          label: LIKED_VIEW_LABEL[v],
          checked: prefs.view === v,
          onSelect: () => setPref({ view: v }),
        })),
      ],
      el,
    );

  const openFind = useCallback(() => {
    setFindOpen(true);
    findRef.current?.focus();
  }, []);
  usePageFind(empty ? null : openFind);
  // Back to the playing song: the table scrolls to it, or the finder and
  // chip that hide it are cleared.
  const tableReveal = useRef<((trackId: string) => boolean) | null>(null);
  const reveal = useCallback((trackId: string) => {
    if (tableReveal.current?.(trackId)) return;
    setQuery("");
    setChip(null);
  }, []);
  usePageReveal(reveal);

  if (error && !data) return <LoadError what="Liked Songs" error={error} onRetry={retry} />;

  return (
    <div className="detail liked">
      <header ref={header} className={`detail-header ${compact ? "detail-header--compact" : ""}`}>
        <div className="detail-header__art">
          <LikedCover size={232} className="liked__cover" />
        </div>
        <div ref={column} className="detail-header__text">
          <span className="detail-header__type">Playlist</span>
          <h1 className="detail-header__title" data-step={step}>
            {TITLE}
          </h1>
          {empty ? null : data ? (
            <div className="detail-header__meta">
              {owner && <span className="detail-header__artist">{owner}</span>}
              {owner && <span>•</span>}
              <span>{plural(counted.length, "song")},</span>
              <span className="detail-header__muted">{formatLongDuration(counted.reduce((s, t) => s + t.durationMs, 0))}</span>
            </div>
          ) : (
            <Skeleton width="30%" height={14} />
          )}
        </div>
      </header>
      <div className="detail-body">
        <div className="action-row">
          {!empty && (
            <>
              <IconButton
                label={isThisPlaying ? `Pause ${TITLE}` : `Play ${TITLE}`}
                variant="accent"
                size="xl"
                text={isThisPlaying ? "Pause" : "Play"}
                disabled={!data}
                icon={isThisPlaying ? <Pause size={24} fill="currentColor" /> : <Play size={24} fill="currentColor" className="nudge-right" />}
                onClick={play}
              />
              <ContextShuffleButton id={LIKED_ID} kind="liked" name={TITLE} />
              <DownloadButton kind="liked" id="" title={TITLE} disabled={!data} />
              <div className="liked__tools">
                <div className={`liked-find ${findOpen ? "is-open" : ""}`}>
                  {findOpen ? (
                    <>
                      <Search size={16} aria-hidden="true" className="liked-find__icon" />
                      <input
                        ref={findRef}
                        autoFocus
                        type="search"
                        className="liked-find__input"
                        aria-label="Find in Liked Songs"
                        placeholder="Find in Liked Songs"
                        value={query}
                        spellCheck={false}
                        autoComplete="off"
                        onChange={(e) => setQuery(e.target.value)}
                        onBlur={() => !query && setFindOpen(false)}
                        onKeyDown={(e) => {
                          if (e.key === "Escape") {
                            e.preventDefault();
                            e.stopPropagation();
                            setQuery("");
                          }
                        }}
                      />
                    </>
                  ) : (
                    <IconButton label="Find in Liked Songs" icon={<Search size={18} />} onClick={openFind} />
                  )}
                </div>
                <button type="button" className="pl-sort liked__sort" aria-haspopup="menu" aria-label={`Sort and view: ${LIKED_SORT_LABEL[prefs.sort]}, ${LIKED_VIEW_LABEL[prefs.view]}`} onClick={(e) => sortMenu(e.currentTarget)}>
                  <span>{LIKED_SORT_LABEL[prefs.sort]}</span>
                  {prefs.view === "list" ? <List size={16} aria-hidden="true" /> : <Rows3 size={16} aria-hidden="true" />}
                  <ArrowDownUp size={14} aria-hidden="true" className="liked__sort-arrows" />
                </button>
              </div>
            </>
          )}
        </div>
        {!empty && chips.length > 0 && (
          <ChipRow label="Filter by genre or mood" className="liked__chips" scroll>
            {activeChip ? (
              <>
                <button type="button" className="chip chip--clear liked__chip-clear" aria-label="Clear filter" onClick={() => setChip(null)}>
                  <X size={16} aria-hidden="true" />
                </button>
                <button type="button" className="chip is-selected" aria-pressed="true" onClick={() => setChip(null)}>
                  {activeChip}
                </button>
              </>
            ) : (
              chips.map((c) => (
                <button key={c.tag} type="button" className="chip" aria-pressed="false" onClick={() => setChip(c.tag)}>
                  {c.tag}
                </button>
              ))
            )}
          </ChipRow>
        )}
        {!empty && <FacetChips view="songs" options={facetOpts} chips={facets} onChange={setFacets} />}
        {empty ? (
          <div className="empty-state liked__empty">
            <Heart size={40} strokeWidth={1.5} aria-hidden="true" />
            <h2 className="empty-state__title">Songs you like show up here</h2>
            <p className="empty-state__text">Save a song with the plus next to it, and it lands at the top of this list.</p>
            <Button variant="primary" onClick={() => nav.go({ name: "search" })}>
              Find songs
            </Button>
          </div>
        ) : data && rows.length === 0 && chipsOn(facets) && !query.trim() && !activeChip ? (
          <FacetsEmpty view="songs" onClear={() => setFacets(NO_CHIPS)} />
        ) : data && rows.length === 0 ? (
          <div className="empty-state liked__none" role="status">
            <h2 className="empty-state__title">{query.trim() ? `Couldn't find "${query.trim()}"` : "No liked songs here"}</h2>
            <p className="empty-state__text">
              {query.trim() ? "Check the spelling, or try fewer or different words." : "Clear the filter to see all your liked songs."}
            </p>
          </div>
        ) : (
          <LikedTable
            rows={rows}
            loading={!data}
            view={prefs.view}
            nowTrackId={nowTrackId}
            playing={status === "playing"}
            liked={liked}
            entering={entering}
            onPlay={(t) => playFrom(t.id)}
            revealRef={tableReveal}
          />
        )}
      </div>
    </div>
  );
}

/** The song table, virtualised against the main scroller (only rows on screen exist). */
function LikedTable({
  rows,
  loading,
  view,
  nowTrackId,
  playing,
  liked,
  entering,
  onPlay,
  revealRef,
}: {
  rows: Track[];
  loading: boolean;
  view: LikedView;
  nowTrackId: string | undefined;
  playing: boolean;
  liked: ReadonlySet<string>;
  entering: ReadonlySet<string>;
  onPlay: (t: Track) => void;
  revealRef?: MutableRefObject<((trackId: string) => boolean) | null>;
}) {
  const root = useRef<HTMLDivElement>(null);
  const [scrollEl, setScrollEl] = useState<HTMLElement | null>(null);
  const [offset, setOffset] = useState(0);
  const rowH = ROW_H[view];

  useLayoutEffect(() => {
    const el = root.current;
    if (!el) return;
    const sc = el.closest<HTMLElement>(".main__scroll") ?? (document.scrollingElement as HTMLElement | null);
    setScrollEl(sc);
    const measure = () => sc && setOffset(el.getBoundingClientRect().top - sc.getBoundingClientRect().top + sc.scrollTop);
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    if (sc) ro.observe(sc);
    return () => ro.disconnect();
  }, [loading]);

  const v = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollEl,
    estimateSize: () => rowH,
    overscan: 8,
    scrollMargin: offset,
    initialRect: { width: 1024, height: 900 },
    observeElementRect: observeNonEmptyRect,
  });
  useEffect(() => {
    v.measure();
  }, [rowH, v]);
  // The whole list for select-all, ranges, Home / End and type-ahead.
  const model = useMemo(() => ({ keys: rows.map((t) => t.id), tracks: rows }), [rows]);
  const scrollToIndex = useCallback((i: number) => v.scrollToIndex(i, { align: "auto" }), [v]);
  useEffect(() => {
    if (!revealRef) return;
    revealRef.current = (trackId) => {
      const i = rows.findIndex((t) => t.id === trackId);
      if (i < 0) return false;
      v.scrollToIndex(i, { align: "center" });
      return true;
    };
    return () => {
      revealRef.current = null;
    };
  }, [revealRef, rows, v]);

  return (
    <div ref={root} className={`liked__list liked__list--${view}`}>
      <TrackList
        label="Liked Songs"
        addedColumn
        rowCount={loading ? undefined : rows.length}
        model={loading ? undefined : model}
        scrollToIndex={scrollToIndex}
      >
        {loading ? (
          Array.from({ length: 8 }, (_, i) => <TrackRowSkeleton key={i} showArt />)
        ) : (
          <div className="liked__rows" style={{ height: v.getTotalSize(), position: "relative" }}>
            {v.getVirtualItems().map((r) => {
              const t = rows[r.index];
              return (
                <div
                  key={t.id}
                  className="liked__vrow"
                  style={{ position: "absolute", top: 0, left: 0, width: "100%", height: rowH, transform: `translateY(${r.start - v.options.scrollMargin}px)` }}
                >
                  <TrackRow
                    track={t}
                    index={r.index + 1}
                    rowIndex={r.index + 2}
                    showArt={view === "list"}
                    active={t.id === nowTrackId}
                    playing={t.id === nowTrackId && playing}
                    likedQuiet
                    added={addedLabel(t.ratedAt)}
                    entering={entering.has(t.id)}
                    dimmed={!liked.has(t.id)}
                    onPlay={() => onPlay(t)}
                  />
                </div>
              );
            })}
          </div>
        )}
      </TrackList>
    </div>
  );
}
