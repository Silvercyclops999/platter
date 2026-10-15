import { Clock3, Ellipsis, Pause, Play, X } from "lucide-react";
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type MouseEvent,
  type ReactNode,
} from "react";
import { trackCredits } from "../lib/credits";
import { formatDuration, trackArtistLine } from "../lib/format";
import { isMod } from "../lib/platform";
import type { Track } from "../lib/types";
import { isUnavailable, useApp } from "../stores/app";
import { endSongDrag, startSongDrag, tracksDrag } from "../stores/drag";
import { useSettings } from "../stores/settings";
import { likeMenuLabel, toggleLiked } from "../stores/likes";
import { lyricsEditLabel, openLyricsEditor } from "../stores/lyricsEditor";
import { nav, useRouter } from "../stores/router";
import { player } from "../stores/player";
import { toast } from "../stores/toasts";
import type { MenuItem } from "../stores/ui";
import { Art } from "./Art";
import { ArtistLinks, TextLink, linkFeaturedNow } from "./ArtistLinks";
import { QualityBadge } from "./Badge";
import { ArchiveQualityBadge, archiveQualityLabel, ExplicitBadge, VersionBadge, versionLabel } from "./FileTagBadges";
import { useTrackBadges } from "../stores/fileTags";
import { openContextMenu } from "./ContextMenu";
import { addToPlaylistItem } from "./PlaylistPicker";
import { isTypeAheadKey, linkStep, rowStep, typeAhead } from "./listKeys";
import { SaveButton } from "./SaveButton";
import { DownloadedMark } from "./DownloadButton";
import { useIsDownloaded } from "../stores/downloads";
import { shareItem } from "./ShareMenu";
import { sessionItems } from "../stores/social";
import { bookmarkTrackItem, dontPlayArtistItem } from "../stores/curate";
import { yourStatsItem } from "../stores/stats";
import { Skeleton } from "./Skeleton";
import { useOwnDoubleClick } from "../lib/doubleClick";
import { openVersions, otherVersionsItem } from "../stores/archive";
import { showVersionLink, versionLinkText } from "../pages/era";
import "./archive.css";

/** Whether the album page for `albumId` is the one showing. */
export function onAlbumPage(albumId: string): boolean {
  const { entries, index } = useRouter.getState();
  const r = entries[index]?.route;
  return r?.name === "album" && r.id === albumId;
}

/**
 * One "Go to <name>" per artist in the track's line that has a page (every
 * credited name while "Link featured artists" is on), in line order. When
 * none does, the album artist.
 */
export function goToArtistItems(track: Track): MenuItem[] {
  const linked = trackCredits(track, useApp.getState().artistLookup, linkFeaturedNow()).filter((c) => c.id);
  const targets = linked.length ? linked.map((c) => ({ name: c.name, id: c.id! })) : [{ name: track.artistTitle, id: track.artistId }];
  return targets.map((a) => ({ label: `Go to ${a.name}`, onSelect: () => nav.go({ name: "artist", id: a.id }) }));
}

/** "Add to Liked Songs" / "Remove from Liked Songs" for songs (the label follows their state). */
export function likeItem(tracks: () => Track[]): MenuItem {
  return {
    // Read when the menu shows: rows build their menu ahead of time.
    get label() {
      return likeMenuLabel(tracks().map((t) => t.id));
    },
    onSelect: () => void toggleLiked(tracks()),
  };
}

/**
 * Standard track context menu, also used for the now-playing track. Order:
 * add to playlist, the like toggle, queue actions, then where to go, then copy. "Go to album"
 * is left out on that album's own page. `afterPlaylist` items (remove from
 * this playlist) go right after "Add to playlist", `afterQueue` items (e.g.
 * remove from queue) right after "Add to queue"; pages can also prepend or
 * append items.
 */
export function trackMenuItems(
  track: Track,
  extra: { before?: MenuItem[]; afterPlaylist?: MenuItem[]; afterQueue?: MenuItem[]; after?: MenuItem[] } = {},
): MenuItem[] {
  return [
    ...(extra.before ?? []),
    addToPlaylistItem(() => [track.id]),
    likeItem(() => [track]),
    ...(extra.afterPlaylist ?? []),
    { label: "Add to queue", onSelect: () => void player.addToQueue([track]) },
    ...(extra.afterQueue ?? []),
    ...sessionItems(() => [track.id], () => void player.playTracks([track.id], track.id)),
    ...goToArtistItems(track),
    { label: "Go to album", when: () => !onAlbumPage(track.albumId), onSelect: () => nav.go({ name: "album", id: track.albumId }) },
    { label: "Go to song radio", onSelect: () => nav.go({ name: "radio", seed: `track:${track.id}` }) },
    ...otherVersionsItem(track),
    bookmarkTrackItem(track),
    yourStatsItem("song", track.id, track.title),
    dontPlayArtistItem({ id: track.artistId, title: track.artistTitle }),
    {
      // Read when the menu shows: lyrics may have been saved since the row built it.
      get label() {
        return lyricsEditLabel(track.id);
      },
      onSelect: () => openLyricsEditor(track),
    },
    shareItem("track", track.id, track.thumb, { title: track.title, subtitle: trackArtistLine(track), durationMs: track.durationMs }),
    {
      label: "Copy song title",
      onSelect: () => {
        navigator.clipboard?.writeText(`${track.title} - ${trackArtistLine(track)}`).then(
          () => toast.info("Copied to clipboard"),
          () => toast.error("Couldn't copy"),
        );
      },
    },
    ...(extra.after ?? []),
  ];
}

/** Songs for "Add to queue": the rows' tracks when they're all known, else bare ids (no cover). */
export function queueable(trackIds: string[], tracks: Track[] = []): { id: string; thumb: string | null }[] {
  return tracks.length === trackIds.length ? tracks : trackIds.map((id) => ({ id, thumb: null }));
}

/** Menu for several selected rows: only actions that make sense for all of them. */
export function multiTrackMenuItems(trackIds: string[], tracks: Track[] = []): MenuItem[] {
  return [
    { label: "Add to queue", onSelect: () => void player.addToQueue(queueable(trackIds, tracks)) },
    ...(tracks.length ? [likeItem(() => tracks)] : []),
    addToPlaylistItem(() => trackIds),
  ];
}

/** The tracks for a multi-selection's track ids, looked up in the rows on screen. */
const tracksIn = new WeakMap<HTMLElement, Track>();

// ---------------------------------------------------------------- selection

interface Selection {
  selected: ReadonlySet<string>;
  /** Plain click selects one; Ctrl/⌘ toggles; Shift extends from the anchor. */
  click: (key: string, mode: "one" | "toggle" | "range") => void;
  selectOnly: (key: string) => void;
  selectAll: () => void;
  clear: () => void;
  /** Menu for the whole selection, or null when the row isn't part of a multi-selection. */
  multiMenu: (key: string) => { label: string; items: MenuItem[] } | null;
  /** The selected rows' tracks in list order. */
  pickedTracks: () => Track[];
  /** The list is one Tab stop: the row that has tabindex 0. */
  tabStop: string | null;
  setTabStop: (key: string) => void;
}

const SelectionContext = createContext<Selection | null>(null);

function rowKeys(root: HTMLElement | null): HTMLElement[] {
  return Array.from(root?.querySelectorAll<HTMLElement>(".track-row[data-row-key]") ?? []);
}

function useSelection(
  root: React.RefObject<HTMLDivElement | null>,
  buildMulti: (keys: string[], trackIds: string[]) => MenuItem[],
): Selection {
  const [selected, setSelected] = useState<ReadonlySet<string>>(() => new Set());
  const [tabStop, setTabStop] = useState<string | null>(null);
  const anchor = useRef<string | null>(null);

  const ordered = useCallback(() => rowKeys(root.current).map((el) => el.dataset.rowKey!), [root]);

  const click = useCallback(
    (key: string, mode: "one" | "toggle" | "range") => {
      setSelected((prev) => {
        if (mode === "toggle") {
          const next = new Set(prev);
          if (next.has(key)) next.delete(key);
          else next.add(key);
          anchor.current = key;
          return next;
        }
        if (mode === "range" && anchor.current) {
          const keys = ordered();
          const a = keys.indexOf(anchor.current);
          const b = keys.indexOf(key);
          if (a >= 0 && b >= 0) return new Set(keys.slice(Math.min(a, b), Math.max(a, b) + 1));
        }
        anchor.current = key;
        return new Set([key]);
      });
    },
    [ordered],
  );

  const clear = useCallback(() => {
    anchor.current = null;
    setSelected((prev) => (prev.size ? new Set() : prev));
  }, []);

  // Clicking anywhere outside the list (but not in its menu) clears the selection.
  useEffect(() => {
    if (!selected.size) return;
    const onDown = (e: globalThis.MouseEvent) => {
      const t = e.target as Element | null;
      if (!t || root.current?.contains(t) || t.closest?.(".context-menu")) return;
      clear();
    };
    window.addEventListener("mousedown", onDown, true);
    return () => window.removeEventListener("mousedown", onDown, true);
  }, [selected, root, clear]);

  return useMemo<Selection>(
    () => ({
      selected,
      click,
      clear,
      selectOnly: (key) => {
        anchor.current = key;
        setSelected((prev) => (prev.size === 1 && prev.has(key) ? prev : new Set([key])));
      },
      selectAll: () => setSelected(new Set(ordered())),
      multiMenu: (key) => {
        if (selected.size < 2 || !selected.has(key)) return null;
        const rows = rowKeys(root.current).filter((el) => selected.has(el.dataset.rowKey!));
        const keys = rows.map((el) => el.dataset.rowKey!);
        const ids = rows.map((el) => el.dataset.trackId!);
        return { label: `Options for ${rows.length} songs`, items: buildMulti(keys, ids) };
      },
      pickedTracks: () =>
        rowKeys(root.current)
          .filter((el) => selected.has(el.dataset.rowKey!))
          .map((el) => tracksIn.get(el))
          .filter((t): t is Track => !!t),
      tabStop,
      setTabStop,
    }),
    [selected, click, clear, ordered, root, buildMulti, tabStop],
  );
}

// ---------------------------------------------------------------- row

export interface TrackRowProps {
  track: Track;
  /** Number shown in the first column (track number or list position). */
  index: number;
  onPlay: () => void;
  /** Identity for selection; defaults to the track id (pass a queue item id
   *  or position when the same track can appear twice). */
  rowKey?: string;
  /** This row is the current track. */
  active?: boolean;
  /** …and it is playing (shows the equalizer glyph). */
  playing?: boolean;
  showArt?: boolean;
  showAlbum?: boolean;
  /** Show the artist line under the title (off on single-artist albums when equal). */
  showArtist?: boolean;
  /** Album tables: a play count column in place of the quality column. */
  showPlays?: boolean;
  menuItems?: MenuItem[];
  /** Hover action that removes the row (queue). */
  onRemove?: () => void;
  /** Extra trailing content. */
  trailing?: ReactNode;
  /** A small mark before the artist line (the queue's smart shuffle sparkle). */
  subBadge?: ReactNode;
  /** Added to the row's accessible name, after a comma. */
  labelNote?: string;
  compact?: boolean;
  /** Highlighted by keyboard navigation that lives elsewhere (search box arrows). */
  selected?: boolean;
  /** Liked Songs page: the like tick shows on hover only. */
  likedQuiet?: boolean;
  /** "Date added" column text (in place of the quality badges). */
  added?: string;
  /** Fades in (a new like at the top of Liked Songs). */
  entering?: boolean;
  /** Dimmed (unliked on the Liked Songs page, until you leave it). */
  dimmed?: boolean;
  /** Reorder within the list (custom order playlists, the queue). Songs drag
   *  onto playlists and the queue anyway while that setting is on. */
  draggable?: boolean;
  onDragStart?: (e: React.DragEvent) => void;
  onDragOver?: (e: React.DragEvent) => void;
  onDragLeave?: (e: React.DragEvent) => void;
  onDragEnd?: (e: React.DragEvent) => void;
  onDrop?: (e: React.DragEvent) => void;
  /** Drop indicator line above/below this row while dragging. */
  dropEdge?: "before" | "after" | null;
  /** Position in the whole list for screen readers (header row is 1), when
   *  only some rows are on the page (virtual lists); otherwise the list
   *  numbers its rows itself. */
  rowIndex?: number;
}

/**
 * One track line: number ⇄ play on hover (pause on the playing row), art,
 * title + artist, album, badges, duration and hover actions. Click selects
 * (Ctrl/⌘ toggles, Shift extends), double-click or Enter plays, right-click
 * or Shift+F10 opens the menu for the row or the whole selection.
 */
export function TrackRow({
  track,
  index,
  onPlay,
  rowKey,
  active,
  playing,
  showArt = false,
  showAlbum = true,
  showArtist = true,
  showPlays = false,
  menuItems,
  onRemove,
  trailing,
  subBadge,
  labelNote,
  compact,
  selected,
  likedQuiet,
  added,
  entering,
  dimmed,
  draggable,
  onDragStart,
  onDragOver,
  onDragLeave,
  onDragEnd,
  onDrop,
  dropEdge,
  rowIndex,
}: TrackRowProps) {
  const sel = useContext(SelectionContext);
  const key = rowKey ?? track.id;
  const picked = !!sel?.selected.has(key);
  const items = menuItems ?? trackMenuItems(track);
  // Dimmed while only music on this computer can play and this isn't.
  const unavailable = useApp((s) => isUnavailable(track.id, s));
  const badges = useTrackBadges(track.id);
  const downloaded = useIsDownloaded(track.id);
  const artists = trackArtistLine(track);
  const label = `${track.title}${badges.explicit ? ", explicit" : ""} by ${artists}${downloaded ? ", downloaded" : ""}${unavailable ? ", not available right now" : ""}${labelNote ? `, ${labelNote}` : ""}`;
  // The hover button on the current track pauses / resumes instead of restarting.
  const playButton = () => (active ? player.toggle() : onPlay());
  const PlayIcon = active && playing ? Pause : Play;
  const playLabel = `${active && playing ? "Pause" : "Play"} ${track.title} by ${artists}`;
  const songDrags = useSettings((s) => s.settings?.dragAndDrop !== false);
  const versionsOn = useSettings((s) => s.settings?.showVersionCounts);
  const versions = showVersionLink(track, versionsOn) ? (
    <button
      type="button"
      tabIndex={-1}
      className="versions-link"
      aria-label={`${versionLinkText(track.otherVersions!)} of ${track.title}`}
      onClick={(e) => {
        e.stopPropagation();
        openVersions(track);
      }}
    >
      {versionLinkText(track.otherVersions!)}
    </button>
  ) : null;

  // A drag takes the whole selection when this row is part of it, otherwise
  // just this row, which gets the selected highlight for the drag.
  const dragStart = (e: React.DragEvent) => {
    const many = picked && (sel?.selected.size ?? 0) > 1 ? sel!.pickedTracks() : [];
    if (!many.length) sel?.selectOnly(key);
    startSongDrag(e, tracksDrag(many.length ? many : [track]));
    onDragStart?.(e);
  };
  const dragEnd = (e: React.DragEvent) => {
    endSongDrag();
    onDragEnd?.(e);
  };

  const openMenu = (at: { clientX: number; clientY: number } | HTMLElement) => {
    const multi = sel?.multiMenu(key);
    if (multi) {
      openContextMenu(multi.label, multi.items, at);
      return;
    }
    sel?.selectOnly(key);
    openContextMenu(`Options for ${track.title}`, items, at);
  };

  const dbl = useOwnDoubleClick();
  const onClick = (e: MouseEvent<HTMLDivElement>) => {
    if (!sel || (e.target as HTMLElement).closest("button")) return;
    sel.click(key, e.shiftKey ? "range" : isMod(e) ? "toggle" : "one");
  };
  const onContextMenu = (e: MouseEvent<HTMLDivElement>) => {
    e.preventDefault();
    openMenu(e);
  };
  // Keyboard: the list is one Tab stop. Up / Down / Home / End move between
  // rows and the selection follows (Shift extends it); Right steps into the
  // row's links and Left / Right walk them; letters jump by title.
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const row = e.currentTarget;
    const target = e.target as HTMLElement;
    const plain = !e.altKey && !isMod(e);
    if (plain && !e.shiftKey) {
      const link = linkStep(row, target, e.key);
      if (link) {
        e.preventDefault();
        link.focus();
        return;
      }
    }
    const rowMove = plain && (e.key === "ArrowDown" || e.key === "ArrowUp" || (!e.shiftKey && (e.key === "Home" || e.key === "End")));
    if (target !== row && !rowMove) return;
    if (e.key === "Enter") {
      e.preventDefault();
      onPlay();
    } else if (e.key === "ContextMenu" || (e.key === "F10" && e.shiftKey)) {
      e.preventDefault();
      openMenu(row);
    } else if (e.key === "Escape" && sel?.selected.size) {
      e.preventDefault();
      sel.clear();
    } else if (isMod(e) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "a" && sel) {
      e.preventDefault();
      sel.selectAll();
    } else if (rowMove) {
      const next = rowStep(row, e.key);
      if (!next) return;
      e.preventDefault();
      next.focus();
      if (!sel) return;
      if (e.shiftKey) {
        if (!sel.selected.size) sel.click(key, "one");
        sel.click(next.dataset.rowKey!, "range");
      } else sel.click(next.dataset.rowKey!, "one");
    } else if (sel && isTypeAheadKey(e)) {
      const next = typeAhead(row, e.key, (el) => tracksIn.get(el)?.title ?? "");
      if (!next) return;
      e.preventDefault();
      if (next !== row) next.focus();
      sel.click(next.dataset.rowKey!, "one");
    }
  };

  return (
    <div
      role="row"
      tabIndex={sel ? (sel.tabStop === key ? 0 : -1) : 0}
      aria-label={label}
      aria-current={active ? "true" : undefined}
      aria-selected={sel ? picked || selected === true : undefined}
      aria-rowindex={rowIndex}
      data-row-key={key}
      data-track-id={track.id}
      data-selected={picked || undefined}
      title={unavailable ? "Not available right now" : undefined}
      ref={(el) => {
        if (el) tracksIn.set(el, track);
      }}
      className={`track-row ${active ? "is-active" : ""} ${playing ? "is-playing" : ""} ${unavailable ? "is-unavailable" : ""} ${entering ? "is-entering" : ""} ${dimmed ? "is-dimmed" : ""} ${added !== undefined ? "track-row--added" : ""} ${showAlbum ? "" : "track-row--no-album"} ${showPlays && !compact ? "track-row--plays" : ""} ${compact ? "track-row--compact" : ""} ${selected ? "is-selected" : ""} ${picked ? "is-picked" : ""} ${dropEdge ? `is-drop-${dropEdge}` : ""}`}
      onClick={onClick}
      onMouseDown={dbl.onMouseDown}
      onDoubleClick={(e) => {
        // The second click of a card double-click that opened this page.
        if (dbl.stray()) return;
        if ((e.target as HTMLElement).closest("button")) return;
        onPlay();
      }}
      onKeyDown={onKeyDown}
      onFocus={(e) => {
        if (e.target === e.currentTarget && sel && sel.tabStop !== key) sel.setTabStop(key);
      }}
      onContextMenu={onContextMenu}
      draggable={draggable || songDrags || undefined}
      onDragStart={draggable || songDrags ? dragStart : undefined}
      // Enter as well as over: the drop line shows at once.
      onDragEnter={onDragOver}
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDragEnd={dragEnd}
      onDrop={onDrop}
    >
      {!compact && (
        <div className="track-row__num" role="gridcell">
          {playing ? (
            <span className="eq" aria-hidden="true">
              <i />
              <i />
              <i />
            </span>
          ) : (
            <span className="track-row__index tabular">{index}</span>
          )}
          <button
            type="button"
            tabIndex={-1}
            className="track-row__play"
            aria-label={playLabel}
            onClick={(e) => {
              e.stopPropagation();
              playButton();
            }}
          >
            <PlayIcon size={14} fill="currentColor" aria-hidden="true" />
          </button>
        </div>
      )}
      <div className="track-row__main" role="gridcell">
        {(showArt || compact) && (
          <span className="track-row__art">
            <Art path={track.thumb} size={compact ? 48 : 40} />
            {compact && (
              <button
                type="button"
                tabIndex={-1}
                className={`track-row__art-play ${playing ? "is-playing" : ""}`}
                aria-label={playLabel}
                onClick={(e) => {
                  e.stopPropagation();
                  playButton();
                }}
              >
                {playing ? (
                  <span className="eq eq--on-art" aria-hidden="true">
                    <i />
                    <i />
                    <i />
                  </span>
                ) : null}
                <PlayIcon size={14} fill="currentColor" aria-hidden="true" className="track-row__art-glyph" />
              </button>
            )}
          </span>
        )}
        <div className="track-row__text">
          {versionLabel(badges.version) || archiveQualityLabel(badges.quality, track) || versions || ((badges.explicit || downloaded) && !showArtist) ? (
            <div className="track-row__title-line">
              <div className="track-row__title truncate">{track.title}</div>
              {downloaded && !showArtist && <DownloadedMark trackId={track.id} />}
              {badges.explicit && !showArtist && <ExplicitBadge />}
              <VersionBadge version={badges.version} />
              <ArchiveQualityBadge track={track} />
              {versions}
            </div>
          ) : (
            <div className="track-row__title truncate">{track.title}</div>
          )}
          {showArtist &&
            (badges.explicit || downloaded || subBadge ? (
              <div className="track-row__sub">
                {subBadge}
                {downloaded && <DownloadedMark trackId={track.id} />}
                {badges.explicit && <ExplicitBadge />}
                <ArtistLinks track={track} className="track-row__artist truncate" linkTabIndex={-1} />
              </div>
            ) : (
              <ArtistLinks track={track} className="track-row__artist truncate" linkTabIndex={-1} />
            ))}
        </div>
      </div>
      {showAlbum && !compact && (
        <div className="track-row__album truncate" role="gridcell">
          <TextLink tabIndex={-1} onActivate={() => nav.go({ name: "album", id: track.albumId })}>
            {track.albumTitle}
          </TextLink>
        </div>
      )}
      {!compact &&
        (added !== undefined ? (
          <div className="track-row__added" role="gridcell">
            {added}
          </div>
        ) : showPlays ? (
          <div className="track-row__plays tabular" role="gridcell">
            {track.viewCount.toLocaleString()}
          </div>
        ) : (
          <div className="track-row__badges" role="gridcell">
            <QualityBadge track={track} />
          </div>
        ))}
      <div className="track-row__end" role="gridcell">
        {!compact && <SaveButton track={track} place="row" quietWhenLiked={likedQuiet} />}
        <span className="track-row__duration tabular">{formatDuration(track.durationMs)}</span>
        {trailing}
        {onRemove && (
          <button
            type="button"
            tabIndex={-1}
            className="track-row__hover-btn"
            aria-label={`Remove ${track.title} from queue`}
            onClick={(e) => {
              e.stopPropagation();
              onRemove();
            }}
          >
            <X size={16} aria-hidden="true" />
          </button>
        )}
        <button
          type="button"
          tabIndex={-1}
          className="track-row__hover-btn"
          aria-label={`More options for ${track.title} by ${artists}`}
          onClick={(e) => {
            e.stopPropagation();
            openMenu(e.currentTarget);
          }}
        >
          <Ellipsis size={16} aria-hidden="true" />
        </button>
      </div>
    </div>
  );
}

/**
 * Container for TrackRows: header row, arrow-key navigation and selection
 * scope. For screen readers it is a grid named `label` with its row and
 * column counts, each row numbered and marked selected or not.
 */
export function TrackList({
  children,
  label,
  showAlbum = true,
  showPlays = false,
  header = true,
  addedColumn = false,
  rowCount,
  multiMenu = (_keys: string[], ids: string[], tracks: Track[]) => multiTrackMenuItems(ids, tracks),
}: {
  children: ReactNode;
  label: string;
  showAlbum?: boolean;
  showPlays?: boolean;
  header?: boolean;
  /** The header names a "Date added" column (rows pass `added`). */
  addedColumn?: boolean;
  /** Menu for a multi-selection: row keys and track ids in list order. */
  multiMenu?: (keys: string[], trackIds: string[], tracks: Track[]) => MenuItem[];
  /** Total songs when only some rows are on the page (virtual lists; rows then pass `rowIndex`). */
  rowCount?: number;
}) {
  const root = useRef<HTMLDivElement>(null);
  const buildMulti = useRef(multiMenu);
  buildMulti.current = multiMenu;
  const stableBuild = useCallback((k: string[], ids: string[]) => buildMulti.current(k, ids, tracksFor(root.current, ids)), []);
  const selection = useSelection(root, stableBuild);
  // Keep exactly one row as the list's Tab stop: the last focused, else the first.
  const { tabStop, setTabStop } = selection;
  useLayoutEffect(() => {
    const rows = root.current?.querySelectorAll<HTMLElement>(".track-row[data-row-key]");
    if (!rows?.length) return;
    const keys = Array.from(rows, (el) => el.dataset.rowKey!);
    if (!tabStop || !keys.includes(tabStop)) setTabStop(keys[0]);
  });
  // Row and column counts for screen readers ("row 12 of 61").
  useLayoutEffect(() => {
    const grid = root.current;
    if (!grid) return;
    const rows = Array.from(grid.querySelectorAll<HTMLElement>('[role="row"]'));
    const body = rows.filter((r) => !r.classList.contains("track-list__head"));
    if (rowCount === undefined) rows.forEach((r, i) => r.setAttribute("aria-rowindex", String(i + 1)));
    grid.setAttribute("aria-rowcount", String(rowCount === undefined ? rows.length : rowCount + (header ? 1 : 0)));
    const cells = (body.find((r) => r.classList.contains("track-row")) ?? rows[0])?.querySelectorAll(':scope > [role="gridcell"], :scope > [role="columnheader"]').length;
    if (cells) grid.setAttribute("aria-colcount", String(cells));
    // Nothing to read yet (loading): say so rather than show an empty grid.
    if (body.length) grid.removeAttribute("aria-busy");
    else grid.setAttribute("aria-busy", "true");
  });
  return (
    <SelectionContext.Provider value={selection}>
      <div className="track-list" data-tracklist ref={root} role="grid" aria-label={label} aria-multiselectable="true">
        {header && (
          <div className={`track-list__head ${showAlbum ? "" : "track-row--no-album"} ${showPlays ? "track-row--plays" : ""} ${addedColumn ? "track-row--added" : ""}`} role="row">
            <span className="track-row__num" role="columnheader" aria-label="Number">
              #
            </span>
            <span role="columnheader">Title</span>
            {showAlbum && (
              <span className="track-list__album" role="columnheader">
                Album
              </span>
            )}
            {addedColumn ? (
              <span className="track-list__added" role="columnheader">
                Date added
              </span>
            ) : showPlays ? (
              <span className="track-list__plays" role="columnheader">
                Plays
              </span>
            ) : (
              <span className="track-list__badges" role="columnheader">
                <span className="sr-only">Quality</span>
              </span>
            )}
            <span className="track-list__dur" title="Duration" role="columnheader">
              <Clock3 size={16} aria-hidden="true" />
              <span className="sr-only">Duration</span>
            </span>
          </div>
        )}
        <div role="rowgroup">{children}</div>
      </div>
    </SelectionContext.Provider>
  );
}

/** Tracks for ids from the rows under `root` (the multi-selection menu). */
function tracksFor(root: HTMLElement | null, ids: string[]): Track[] {
  const rows = Array.from(root?.querySelectorAll<HTMLElement>(".track-row[data-track-id]") ?? []);
  const byId = new Map<string, Track>();
  for (const el of rows) {
    const t = tracksIn.get(el);
    if (t) byId.set(t.id, t);
  }
  return ids.map((id) => byId.get(id)).filter((t): t is Track => !!t);
}

export function TrackRowSkeleton({ showArt = false, showAlbum = true, showPlays = false }: { showArt?: boolean; showAlbum?: boolean; showPlays?: boolean }) {
  return (
    <div className={`track-row track-row--skeleton ${showAlbum ? "" : "track-row--no-album"} ${showPlays ? "track-row--plays" : ""}`} aria-hidden="true">
      <div className="track-row__num">
        <Skeleton width={12} height={12} />
      </div>
      <div className="track-row__main">
        {showArt && <Skeleton width={40} height={40} />}
        <div className="track-row__text">
          <Skeleton width="60%" height={14} />
          <Skeleton width="35%" height={12} style={{ marginTop: 6 }} />
        </div>
      </div>
      {showAlbum && (
        <div className="track-row__album">
          <Skeleton width="50%" height={12} />
        </div>
      )}
      <div className={showPlays ? "track-row__plays" : "track-row__badges"} />
      <div className="track-row__end">
        <Skeleton width={32} height={12} />
      </div>
    </div>
  );
}
