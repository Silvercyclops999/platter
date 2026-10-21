import { ArrowDownUp, Ellipsis, ListMusic, Pause, Pencil, Play, Search, UserPlus } from "lucide-react";
import { PageMissing } from "../components/PageMissing";
import { useCallback, useEffect, useMemo, useRef, useState, type DragEvent, type KeyboardEvent } from "react";
import { Art } from "../components/Art";
import { IconButton } from "../components/Button";
import { ContextShuffleButton } from "../components/ShuffleButton";
import { openContextMenu } from "../components/ContextMenu";
import { bookmarkPageItem, editSmartPlaylist } from "../stores/curate";
import { shareItem } from "../components/ShareMenu";
import { DownloadButton } from "../components/DownloadButton";
import { downloadMenuItems } from "../stores/downloads";
import { addToPlaylistItems, playlistEditItems } from "../components/PlaylistPicker";
import { Skeleton } from "../components/Skeleton";
import { TrackList, TrackRow, TrackRowSkeleton, likeItem, queueable, trackMenuItems } from "../components/TrackRow";
import { backend } from "../lib/backend";
import { formatLongDuration, plural, trackArtistLine } from "../lib/format";
import type { Playlist, PlaylistPage as PlaylistData, SharedInfo, SharedSummary, Track } from "../lib/types";
import { Faces } from "../components/Faces";
import { bylineText, openCircleDialog, useCircle, useShared } from "../stores/circle";
import { useSettings } from "../stores/settings";
import { useCap } from "../stores/caps";
import { useLoad } from "../lib/useLoad";
import { useArtTint } from "../lib/useTint";
import { usePins } from "../stores/pins";
import { player, usePlayer } from "../stores/player";
import { dragLeft } from "../stores/drag";
import { askRemove, editDetails, removeRows, savePlaylistDetails, usePlaylistUi } from "../stores/playlists";
import { nav } from "../stores/router";
import { toast } from "../stores/toasts";
import type { MenuItem } from "../stores/ui";
import { useContextState } from "./cards";
import { usePageBar, useTitleStep } from "./detail";
import { BringWaiting } from "./BringWaiting";
import { PlaylistFinder } from "./PlaylistFinder";
import { GapMarker, MixPill, MixToggle, SORT_LOCKED, bpmNote, mixMenuItem, sortLockedLabel, useAutomixView, useMix, useMixSwitch, useShuffleAsk } from "./PlaylistMix";
import { Tooltip } from "../components/Tooltip";
import "./playlist.css";
import "./liked.css";
import { matchesFind } from "./liked";
import { usePageFind, usePageReveal } from "../shell/pageFind";
import { failText } from "../lib/errorText";

export type PlaylistSort = "custom" | "title" | "artist" | "album" | "addedBy" | "duration" | "plays" | "recent";
export const SORT_LABEL: Record<PlaylistSort, string> = {
  custom: "Custom order",
  title: "Title",
  artist: "Artist",
  album: "Album",
  addedBy: "Added by",
  duration: "Duration",
  plays: "Most played",
  recent: "Recently played",
};
/** "Added by" only sorts a shared playlist. */
export const sortsFor = (shared: boolean) => (Object.keys(SORT_LABEL) as PlaylistSort[]).filter((s) => shared || s !== "addedBy");

const DRAG_TYPE = "text/x-platter-playlist-rows";

interface Row {
  itemId: string;
  track: Track;
}

/** Rows in a sort; custom order is the playlist's own. `who` names who added a row ("" for the owner). */
export function sortRows<R extends Row>(rows: R[], sort: PlaylistSort, who: (r: R) => string = () => ""): R[] {
  if (sort === "custom") return rows;
  const cmp = (a: string, b: string) => a.localeCompare(b, undefined, { sensitivity: "base" });
  const by: Record<Exclude<PlaylistSort, "custom">, (a: R, b: R) => number> = {
    // The owner's songs first, then each person's, keeping the playlist's order within.
    addedBy: (a, b) => cmp(who(a), who(b)),
    title: (a, b) => cmp(a.track.title, b.track.title),
    artist: (a, b) => cmp(trackArtistLine(a.track), trackArtistLine(b.track)) || cmp(a.track.albumTitle, b.track.albumTitle),
    album: (a, b) => cmp(a.track.albumTitle, b.track.albumTitle) || a.track.discNumber - b.track.discNumber || a.track.trackNumber - b.track.trackNumber,
    duration: (a, b) => a.track.durationMs - b.track.durationMs,
    // Most first; never played goes last.
    plays: (a, b) => b.track.viewCount - a.track.viewCount,
    recent: (a, b) => (b.track.lastViewedAt ?? -1) - (a.track.lastViewedAt ?? -1),
  };
  return [...rows].sort(by[sort]);
}

/**
 * Where moving `moving` rows to insertion point `at` (0..n in `order`) puts
 * them, as the row they go after (null: the top). Undefined when nothing
 * would change.
 */
export function moveAfter(order: string[], moving: string[], at: number): string | null | undefined {
  const set = new Set(moving);
  const rest = order.filter((id) => !set.has(id));
  const before = order.slice(0, at).filter((id) => !set.has(id)).length;
  const after = before > 0 ? rest[before - 1] : null;
  const result = [...rest.slice(0, before), ...order.filter((id) => set.has(id)), ...rest.slice(before)];
  return result.every((id, i) => id === order[i]) ? undefined : after;
}

/** One step up or down for the selected rows (Alt+Shift+arrows). */
export function stepAfter(order: string[], moving: string[], dir: -1 | 1): string | null | undefined {
  const set = new Set(moving);
  const first = order.findIndex((id) => set.has(id));
  if (first < 0) return undefined;
  if (dir < 0) {
    // In front of the nearest row above that isn't moving.
    let p = first - 1;
    while (p >= 0 && set.has(order[p])) p--;
    if (p < 0) return undefined;
    return moveAfter(order, moving, p);
  }
  let last = -1;
  order.forEach((id, i) => set.has(id) && (last = i));
  let n = last + 1;
  while (n < order.length && set.has(order[n])) n++;
  if (n >= order.length) return undefined;
  return moveAfter(order, moving, n + 1);
}

/** Where a dragged row would land over row `index`: before or after it. */
function dropPoint(e: DragEvent, index: number): number {
  const r = (e.currentTarget as Element).getBoundingClientRect();
  return e.clientY > r.top + r.height / 2 ? index + 1 : index;
}

export function PlaylistPage({ id }: { id: string }) {
  // A shared playlist reads again when a member's change lands.
  const circleVersion = useCircle((s) => (s.view.playlists.some((p) => p.id === id) ? s.version : 0));
  const { data, error, retry } = useLoad(() => backend.playlistPage(id), [id, circleVersion]);
  // An edit shows at once; the next load from the core replaces it.
  const [optimistic, setOptimistic] = useState<PlaylistData | null>(null);
  useEffect(() => setOptimistic(null), [data]);
  const shown = optimistic ?? data;
  const pl = shown?.playlist;
  const own = !!pl && !pl.smart;
  const shared = useShared(own ? id : undefined);
  const info = useSharedInfo(own ? id : null, !!shared, data);
  const inviting = useSettings((s) => s.settings?.collabPlaylists !== false);
  useArtTint(null, pl?.thumb);
  const nowTrackId = usePlayer((s) => s.state?.track?.id);
  const status = usePlayer((s) => s.state?.status);
  const { isContext: isThis, playing: isThisPlaying } = useContextState("playlist", id);
  const pinned = usePins((s) => s.isPinned("playlist", id));
  const canCover = useCap("playlistPoster", id);
  const play = () => (isThis ? player.toggle() : player.playPlaylist(id, undefined, undefined, sortedIds()));
  usePageBar(pl?.title, isThisPlaying, play);
  const { header, column, step, compact } = useTitleStep(pl?.title);
  const empty = !!shown && shown.tracks.length === 0;
  const [chosenSort, setSort] = useState<PlaylistSort>("custom");
  // Mix on (here or on another device): the playlist's own order.
  const mixed = useMix(id);
  const sort: PlaylistSort = mixed ? "custom" : chosenSort;
  // The song finder: open by itself on an empty playlist, and it stays open
  // while you add to it.
  const [finderOpen, setFinderOpen] = useState(false);
  const loadedEmpty = !!data && data.tracks.length === 0;
  const [finderFor, setFinderFor] = useState<string | null>(null);
  if (data && finderFor !== id) {
    setFinderFor(id);
    setFinderOpen(loadedEmpty);
  }
  const [insertAt, setInsertAt] = useState<number | null>(null);
  const renaming = usePlaylistUi((s) => s.renameId === id || (!!pl && s.renameId === pl.id));
  const listRef = useRef<HTMLDivElement>(null);

  const rows: Row[] = useMemo(
    () => (shown ? shown.tracks.map((track, i) => ({ track, itemId: shown.itemIds[i] ?? `${track.id}-${i}` })) : []),
    [shown],
  );
  const sorted = useMemo(() => sortRows(rows, sort === "addedBy" && !shared ? "custom" : sort, (r) => info?.addedBy[r.itemId]?.name ?? ""), [rows, sort, shared, info]);
  // Find in this playlist (Ctrl/Cmd+F): narrows the rows as you type.
  const [findOpen, setFindOpen] = useState(false);
  const [query, setQuery] = useState("");
  const findRef = useRef<HTMLInputElement>(null);
  const [findFor, setFindFor] = useState(id);
  if (findFor !== id) {
    setFindFor(id);
    setFindOpen(false);
    setQuery("");
  }
  const finding = !!query.trim();
  const view = useMemo(() => (finding ? sorted.filter((r) => matchesFind(r.track, query)) : sorted), [sorted, finding, query]);
  const openFind = useCallback(() => {
    setFindOpen(true);
    findRef.current?.focus();
  }, []);
  usePageFind(shown && shown.tracks.length ? openFind : null);
  // Back to the playing song: a finder hiding it is cleared.
  const reveal = useCallback(
    (trackId: string) => {
      if (finding && !view.some((r) => r.track.id === trackId)) setQuery("");
    },
    [finding, view],
  );
  usePageReveal(reveal);
  const order = rows.map((r) => r.itemId);
  // Sorted or narrowed on screen: play in the order shown, not the playlist's own.
  const sortedIds = () => (view === rows ? undefined : view.map((r) => r.track.id));
  const have = useMemo(() => new Set(rows.map((r) => r.track.id)), [rows]);
  const canDrag = own && sort === "custom" && rows.length > 1 && !finding;
  // Mix: on here, its switch (asking first when sorted), shuffle asking
  // first, and the rows' tempo and transitions.
  const mixSwitch = useMixSwitch(id, sort !== "custom", () => setSort("custom"));
  const shuffleAsk = useShuffleAsk();
  const mixIds = useMemo(() => (mixed ? sorted.map((r) => r.track.id) : []), [mixed, sorted]);
  const automix = useAutomixView(id, mixed, mixIds);

  // Deleted on another device while open: say so and go Home.
  useEffect(() => {
    if (error?.kind === "notFound" && data) {
      toast.info("This playlist was deleted");
      nav.go({ name: "home" });
    }
  }, [error, data]);

  if (error && !data) return <PageMissing what="playlist" error={error} onRetry={retry} />;

  /** Apply a new row order now, then ask the core. */
  const move = (ids: string[], after: string | null | undefined) => {
    if (after === undefined || !shown || !pl) return;
    const set = new Set(ids);
    const moving = rows.filter((r) => set.has(r.itemId));
    const rest = rows.filter((r) => !set.has(r.itemId));
    const at = after === null ? 0 : rest.findIndex((r) => r.itemId === after) + 1;
    const next = [...rest.slice(0, at), ...moving, ...rest.slice(at)];
    setOptimistic({ ...shown, tracks: next.map((r) => r.track), itemIds: next.map((r) => r.itemId) });
    backend.playlistMove(pl.id, ids, after).catch((e) => {
      setOptimistic(null);
      toast.error(failText(ids.length === 1 ? "Couldn't move the song" : `Couldn't move ${ids.length} songs`, e));
    });
  };

  const removeNow = (ids: string[]) => {
    if (!shown || !pl) return;
    const set = new Set(ids);
    const kept = rows.filter((r) => !set.has(r.itemId));
    setOptimistic({ ...shown, tracks: kept.map((r) => r.track), itemIds: kept.map((r) => r.itemId) });
    void removeRows(pl, ids, order);
  };

  /** Selected rows in list order, or the focused one. */
  const selection = (): string[] => {
    const root = listRef.current;
    if (!root) return [];
    const picked = Array.from(root.querySelectorAll<HTMLElement>(".track-row[data-selected]")).map((el) => el.dataset.rowKey!);
    if (picked.length) return picked;
    const focused = document.activeElement instanceof HTMLElement ? document.activeElement.closest<HTMLElement>(".track-row[data-row-key]") : null;
    return focused && root.contains(focused) ? [focused.dataset.rowKey!] : [];
  };

  const onListKeys = (e: KeyboardEvent) => {
    if (!own) return;
    if (e.altKey && e.shiftKey && (e.key === "ArrowUp" || e.key === "ArrowDown")) {
      e.preventDefault();
      e.stopPropagation();
      if (sort !== "custom" || finding) return;
      const ids = selection();
      if (ids.length) move(ids, stepAfter(order, ids, e.key === "ArrowUp" ? -1 : 1));
      return;
    }
    if ((e.key === "Delete" || e.key === "Backspace") && !e.altKey && !e.ctrlKey && !e.metaKey && pl) {
      const ids = selection();
      if (!ids.length) return;
      e.preventDefault();
      e.stopPropagation();
      askRemove(pl, ids, order);
    }
  };

  const dragProps = (row: Row, index: number) =>
    canDrag
      ? {
          draggable: true,
          dropEdge: insertAt === index ? ("before" as const) : insertAt === index + 1 ? ("after" as const) : null,
          onDragStart: (e: DragEvent) => {
            const picked = selection();
            const ids = picked.includes(row.itemId) ? picked : [row.itemId];
            e.dataTransfer.setData(DRAG_TYPE, JSON.stringify(ids));
            e.dataTransfer.effectAllowed = "move";
          },
          onDragOver: (e: DragEvent) => {
            if (!e.dataTransfer.types.includes(DRAG_TYPE)) return;
            e.preventDefault();
            e.dataTransfer.dropEffect = "move";
            const at = dropPoint(e, index);
            setInsertAt(at);
          },
          onDragEnd: () => setInsertAt(null),
          onDrop: (e: DragEvent) => {
            const raw = e.dataTransfer.getData(DRAG_TYPE);
            setInsertAt(null);
            if (!raw) return;
            e.preventDefault();
            const ids = JSON.parse(raw) as string[];
            move(ids, moveAfter(order, ids, dropPoint(e, index)));
          },
        }
      : {};

  const rowMenu = (row: Row): MenuItem[] =>
    trackMenuItems(row.track, {
      afterPlaylist: own ? [{ label: "Remove from this playlist", onSelect: () => removeNow([row.itemId]) }] : [],
    });

  const multiMenu = (keys: string[], trackIds: string[], tracks: Track[]): MenuItem[] => [
    { label: "Add to queue", onSelect: () => void player.addToQueue(queueable(trackIds, tracks)) },
    likeItem(() => tracks),
    ...(own ? [{ label: "Remove from this playlist", onSelect: () => removeNow(keys) }] : []),
    ...addToPlaylistItems(() => trackIds),
  ];

  const pageMenu = (el: HTMLElement) =>
    pl &&
    openContextMenu(
      `Options for ${pl.title}`,
      [
        { label: "Add to queue", disabled: empty, onSelect: () => void player.addToQueue(shown!.tracks, pl.thumb) },
        ...(pl.smart ? [{ label: "Edit rules", onSelect: () => void editSmartPlaylist(pl.id, pl.title) }] : []),
        ...(!own && !empty ? [mixMenuItem(mixed, mixSwitch.toggle)] : []),
        ...playlistEditItems(pl),
        bookmarkPageItem("playlist", pl.id, pl.title, { thumb: pl.thumb }),
        ...(empty ? [] : downloadMenuItems({ kind: "playlist", id: pl.id, title: pl.title })),
        shareItem("playlist", pl.id, pl.thumb),
        {
          label: pinned ? "Unpin from sidebar" : "Pin to sidebar",
          onSelect: () =>
            usePins.getState().toggle({
              kind: "playlist",
              id: pl.id,
              title: pl.title,
              subtitle: `Playlist · ${pl.trackCount} songs`,
              thumb: pl.thumb,
            }),
        },
      ],
      el,
    );

  const sortMenu = (el: HTMLElement) =>
    openContextMenu(
      "Sort songs",
      sortsFor(!!shared).map((s, i) => ({
        heading: i === 0 ? "Sort by" : undefined,
        label: SORT_LABEL[s],
        checked: sort === s,
        onSelect: () => setSort(s),
      })),
      el,
    );

  return (
    <div className="detail">
      <header ref={header} className={`detail-header ${compact ? "detail-header--compact" : ""}`}>
        <div className={`detail-header__art ${pl ? "detail-header__art--record" : ""}`}>
          {pl && <span className="detail-header__record record-disc" aria-hidden="true" />}
          {pl ? (
            own && canCover ? (
              <button type="button" className="cover-button pl-cover" aria-label="Edit details: choose a cover" onClick={() => editDetails(pl)}>
                <Art path={pl.thumb} size={232} kind="playlist" />
                <span className="pl-cover__hint" aria-hidden="true">
                  <Pencil size={40} />
                  <span>Choose image</span>
                </span>
              </button>
            ) : (
              <Art path={pl.thumb} size={232} kind="playlist" />
            )
          ) : (
            <Skeleton width={232} height={232} />
          )}
        </div>
        <div ref={column} className="detail-header__text">
          {pl ? (
            <>
              <span className="detail-header__type">{pl.smart ? "Smart playlist" : "Playlist"}</span>
              {renaming && own ? (
                <RenameField playlist={pl} step={step} />
              ) : own ? (
                <h1 className="detail-header__title pl-title" data-step={step}>
                  <button type="button" className="pl-title__button" title="Edit details" onClick={() => editDetails(pl)}>
                    {pl.title}
                  </button>
                  {mixed && <span className="pl-mixed"> · Mixed</span>}
                </h1>
              ) : (
                <h1 className="detail-header__title" data-step={step}>
                  {pl.title}
                  {mixed && <span className="pl-mixed"> · Mixed</span>}
                </h1>
              )}
              {pl.summary && <p className="pl-summary">{pl.summary}</p>}
              {shared && <Byline shared={shared} />}
              {!empty && (
                <div className="detail-header__meta">
                  <span>{plural(pl.trackCount, "song")},</span>
                  <span className="detail-header__muted">{formatLongDuration(pl.durationMs)}</span>
                </div>
              )}
            </>
          ) : (
            <>
              <Skeleton width={60} height={12} />
              <Skeleton width="50%" height={48} style={{ margin: "12px 0" }} />
              <Skeleton width="30%" height={14} />
            </>
          )}
        </div>
      </header>
      <div className="detail-body">
        <div className="action-row">
          {!empty && (
            <>
              <IconButton
                label={isThisPlaying ? `Pause ${pl?.title ?? "playlist"}` : `Play ${pl?.title ?? "playlist"}`}
                variant="accent"
                size="xl"
                text={isThisPlaying ? "Pause" : "Play"}
                disabled={!pl}
                icon={isThisPlaying ? <Pause size={24} fill="currentColor" /> : <Play size={24} fill="currentColor" className="nudge-right" />}
                onClick={play}
              />
              <ContextShuffleButton id={id} kind="playlist" name={pl?.title} beforeShuffle={mixed ? shuffleAsk.ask : undefined} />
              {!own && pl && <MixToggle mixed={mixed} onToggle={mixSwitch.toggle} name={pl.title} />}
              <DownloadButton kind="playlist" id={id} title={pl?.title ?? "this playlist"} disabled={!pl} />
            </>
          )}
          {own && pl && inviting && (
            <IconButton
              label={`Invite collaborators to ${pl.title}`}
              size="lg"
              icon={<UserPlus size={22} />}
              onClick={() => openCircleDialog({ kind: "invite", playlistId: pl.id, title: pl.title })}
            />
          )}
          <IconButton
            label={`More options for ${pl?.title ?? "playlist"}`}
            size="lg"
            icon={<Ellipsis size={22} />}
            disabled={!pl}
            onClick={(e) => pageMenu(e.currentTarget)}
          />
          {!empty && pl && (
            <div className={`liked-find pl-find ${findOpen ? "is-open" : ""}`}>
              {findOpen ? (
                <>
                  <Search size={16} aria-hidden="true" className="liked-find__icon" />
                  <input
                    ref={findRef}
                    autoFocus
                    type="search"
                    className="liked-find__input"
                    aria-label={`Find in ${pl.title}`}
                    placeholder={`Find in ${pl.title}`}
                    value={query}
                    spellCheck={false}
                    autoComplete="off"
                    onChange={(e) => setQuery(e.target.value)}
                    onBlur={() => !query && setFindOpen(false)}
                    onKeyDown={(e) => {
                      if (e.key === "Escape") {
                        e.preventDefault();
                        e.stopPropagation();
                        if (query) setQuery("");
                        else setFindOpen(false);
                      }
                    }}
                  />
                </>
              ) : (
                <IconButton label={`Find in ${pl.title}`} icon={<Search size={18} />} onClick={openFind} />
              )}
            </div>
          )}
          {!empty &&
            pl &&
            (mixed ? (
              <Tooltip label={SORT_LOCKED} describe>
                <button type="button" className="pl-sort pl-sort--locked" aria-disabled="true" aria-label={sortLockedLabel(SORT_LABEL[sort])}>
                  <span>{SORT_LABEL[sort]}</span>
                  <ArrowDownUp size={16} aria-hidden="true" />
                </button>
              </Tooltip>
            ) : (
              <button
                type="button"
                className="pl-sort"
                aria-haspopup="menu"
                aria-label={`Sort songs: ${SORT_LABEL[sort]}`}
                onClick={(e) => sortMenu(e.currentTarget)}
              >
                <span>{SORT_LABEL[sort]}</span>
                <ArrowDownUp size={16} aria-hidden="true" />
              </button>
            ))}
        </div>
        {own && pl && !empty && (
          <div className="pl-mix-row">
            <MixPill mixed={mixed} onToggle={mixSwitch.toggle} />
          </div>
        )}
        {empty && pl ? (
          !own && (
            <div className="empty-state">
              <ListMusic size={40} strokeWidth={1.5} aria-hidden="true" />
              <h2 className="empty-state__title">This playlist is empty</h2>
              <p className="empty-state__text">Songs that match it on your server will show up here.</p>
            </div>
          )
        ) : (
          <div ref={listRef} onKeyDownCapture={onListKeys} onDragLeave={(e) => dragLeft(e) && setInsertAt(null)}>
            <TrackList label={pl ? `${pl.title} songs` : "Songs"} multiMenu={multiMenu} byColumn={!!shared}>
              {shown
                ? view.flatMap((r, i) => {
                    const row = (
                      <TrackRow
                        key={r.itemId}
                        rowKey={r.itemId}
                        track={r.track}
                        index={i + 1}
                        showArt
                        active={r.track.id === nowTrackId}
                        playing={r.track.id === nowTrackId && status === "playing"}
                        onPlay={() => player.playPlaylist(id, r.track.id, undefined, sortedIds(), i)}
                        menuItems={rowMenu(r)}
                        addedBy={shared ? (info?.addedBy[r.itemId] ?? null) : undefined}
                        notes={mixed ? bpmNote(automix, r.track.id) : undefined}
                        {...dragProps(r, i)}
                      />
                    );
                    const next = view[i + 1];
                    const gap = mixed && next && !finding ? automix?.gaps[i] : undefined;
                    if (!gap || gap.fromId !== r.track.id || gap.toId !== next.track.id) return [row];
                    return [row, <GapMarker key={`gap-${r.itemId}`} playlistId={id} gap={gap} from={r.track.title} to={next.track.title} />];
                  })
                : Array.from({ length: 8 }, (_, i) => <TrackRowSkeleton key={i} showArt />)}
            </TrackList>
            {shown && finding && view.length === 0 && <p className="pl-find__none">{`Nothing in this playlist matches "${query.trim()}"`}</p>}
          </div>
        )}
        {own && pl && <BringWaiting playlistId={id} />}
        {own && pl && (
          <div className={`pl-more ${empty ? "pl-more--empty" : ""}`}>
            {finderOpen ? (
              <PlaylistFinder playlist={pl} have={have} onClose={() => setFinderOpen(false)} autoFocus={!renaming} />
            ) : (
              <button type="button" className="see-more" onClick={() => setFinderOpen(true)}>
                {empty ? "Find songs for this playlist" : "Find more"}
              </button>
            )}
          </div>
        )}
      </div>
      {mixSwitch.dialog}
      {shuffleAsk.dialog}
    </div>
  );
}

/** Who added what on your shared playlist, read again whenever it changes. */
function useSharedInfo(id: string | null, shared: boolean, data: unknown): SharedInfo | null {
  const version = useCircle((s) => s.version);
  const [info, setInfo] = useState<SharedInfo | null>(null);
  useEffect(() => {
    if (!id || !shared) {
      setInfo(null);
      return;
    }
    let alive = true;
    backend.collabInfo(id).then(
      (i) => alive && setInfo(i),
      () => {},
    );
    return () => {
      alive = false;
    };
  }, [id, shared, version, data]);
  return info;
}

/** "<owner> and <member>" with their faces; opens Playlist permissions. */
export function Byline({ shared }: { shared: SharedSummary }) {
  return (
    <button type="button" className="pl-byline" aria-label={`${bylineText(shared)}. Playlist permissions`} onClick={() => openCircleDialog({ kind: "permissions", playlistId: shared.id })}>
      <Faces people={shared.people} size={24} />
      <span className="pl-byline__names">{bylineText(shared)}</span>
    </button>
  );
}

/** A just-made playlist's name as a field: type to rename, Enter or leaving saves, Escape keeps it. */
function RenameField({ playlist, step }: { playlist: Playlist; step: number }) {
  const [value, setValue] = useState(playlist.title);
  const input = useRef<HTMLInputElement>(null);
  const done = useRef(false);
  useEffect(() => {
    input.current?.focus();
    input.current?.select();
  }, []);
  const finish = (save: boolean) => {
    if (done.current) return;
    done.current = true;
    usePlaylistUi.getState().set({ renameId: null });
    const name = value.trim();
    if (save && name && name !== playlist.title) void savePlaylistDetails(playlist, { title: name });
  };
  return (
    <h1 className="detail-header__title pl-title" data-step={step}>
      <input
        ref={input}
        className="pl-rename"
        aria-label="Playlist name"
        value={value}
        maxLength={100}
        spellCheck={false}
        onChange={(e) => setValue(e.target.value)}
        onBlur={() => finish(true)}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            finish(true);
          } else if (e.key === "Escape") {
            e.preventDefault();
            e.stopPropagation();
            finish(false);
          }
        }}
      />
    </h1>
  );
}
