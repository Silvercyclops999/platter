import { Ellipsis, Heart, Pause, Play, X } from "lucide-react";
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type MouseEvent,
  type ReactNode,
} from "react";
import { formatDuration, trackArtistLine } from "../lib/format";
import { isMod } from "../lib/platform";
import type { Track } from "../lib/types";
import { isUnavailable, useApp } from "../stores/app";
import { nav, useRouter } from "../stores/router";
import { player } from "../stores/player";
import { toast } from "../stores/toasts";
import type { MenuItem } from "../stores/ui";
import { Art } from "./Art";
import { QualityBadge } from "./Badge";
import { openContextMenu } from "./ContextMenu";
import { Skeleton } from "./Skeleton";

/** Whether the album page for `albumId` is the one showing. */
export function onAlbumPage(albumId: string): boolean {
  const { entries, index } = useRouter.getState();
  const r = entries[index]?.route;
  return r?.name === "album" && r.id === albumId;
}

/**
 * Standard track context menu, also used for the now-playing track. Order:
 * queue actions, then where to go, then copy. "Go to album" is left out on
 * that album's own page. `afterQueue` items (e.g. remove from queue) go right
 * after "Add to queue"; pages can also prepend or append items.
 */
export function trackMenuItems(
  track: Track,
  extra: { before?: MenuItem[]; afterQueue?: MenuItem[]; after?: MenuItem[] } = {},
): MenuItem[] {
  return [
    ...(extra.before ?? []),
    { label: "Add to queue", onSelect: () => player.queueAdd([track.id], "end") },
    ...(extra.afterQueue ?? []),
    { label: "Go to artist", onSelect: () => nav.go({ name: "artist", id: track.artistId }) },
    { label: "Go to album", when: () => !onAlbumPage(track.albumId), onSelect: () => nav.go({ name: "album", id: track.albumId }) },
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

/** Menu for several selected rows: only actions that make sense for all of them. */
export function multiTrackMenuItems(trackIds: string[]): MenuItem[] {
  return [{ label: "Add to queue", onSelect: () => player.queueAdd(trackIds, "end") }];
}

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
    }),
    [selected, click, clear, ordered, root, buildMulti],
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
  menuItems?: MenuItem[];
  /** Hover action that removes the row (queue). */
  onRemove?: () => void;
  /** Extra trailing content. */
  trailing?: ReactNode;
  compact?: boolean;
  /** Highlighted by keyboard navigation that lives elsewhere (search box arrows). */
  selected?: boolean;
  draggable?: boolean;
  onDragStart?: (e: React.DragEvent) => void;
  onDragOver?: (e: React.DragEvent) => void;
  onDragLeave?: (e: React.DragEvent) => void;
  onDragEnd?: (e: React.DragEvent) => void;
  onDrop?: (e: React.DragEvent) => void;
  /** Drop indicator line above/below this row while dragging. */
  dropEdge?: "before" | "after" | null;
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
  menuItems,
  onRemove,
  trailing,
  compact,
  selected,
  draggable,
  onDragStart,
  onDragOver,
  onDragLeave,
  onDragEnd,
  onDrop,
  dropEdge,
}: TrackRowProps) {
  const sel = useContext(SelectionContext);
  const key = rowKey ?? track.id;
  const picked = !!sel?.selected.has(key);
  const items = menuItems ?? trackMenuItems(track);
  // Dimmed while only music on this computer can play and this isn't.
  const unavailable = useApp((s) => isUnavailable(track.id, s));
  const label = `${track.title} by ${trackArtistLine(track)}${unavailable ? ", not available right now" : ""}`;
  // The hover button on the current track pauses / resumes instead of restarting.
  const playButton = () => (active ? player.toggle() : onPlay());
  const PlayIcon = active && playing ? Pause : Play;
  const playLabel = `${active && playing ? "Pause" : "Play"} ${track.title}`;

  const openMenu = (at: { clientX: number; clientY: number } | HTMLElement) => {
    const multi = sel?.multiMenu(key);
    if (multi) {
      openContextMenu(multi.label, multi.items, at);
      return;
    }
    sel?.selectOnly(key);
    openContextMenu(`Options for ${track.title}`, items, at);
  };

  const onClick = (e: MouseEvent<HTMLDivElement>) => {
    if (!sel || (e.target as HTMLElement).closest("button")) return;
    sel.click(key, e.shiftKey ? "range" : isMod(e) ? "toggle" : "one");
  };
  const onContextMenu = (e: MouseEvent<HTMLDivElement>) => {
    e.preventDefault();
    openMenu(e);
  };
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.target !== e.currentTarget) return;
    if (e.key === "Enter") {
      e.preventDefault();
      onPlay();
    } else if (e.key === "ContextMenu" || (e.key === "F10" && e.shiftKey)) {
      e.preventDefault();
      openMenu(e.currentTarget);
    } else if (e.key === "Escape" && sel?.selected.size) {
      e.preventDefault();
      sel.clear();
    } else if (isMod(e) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "a" && sel) {
      e.preventDefault();
      sel.selectAll();
    } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      // Move between rows in the same list; the selection follows (Shift extends it).
      const list = e.currentTarget.closest("[data-tracklist]");
      const rows = Array.from(list?.querySelectorAll<HTMLElement>(".track-row[data-row-key]") ?? []);
      const i = rows.indexOf(e.currentTarget);
      const next = rows[i + (e.key === "ArrowDown" ? 1 : -1)];
      if (next) {
        e.preventDefault();
        next.focus();
        if (sel && (e.shiftKey || sel.selected.size)) {
          if (e.shiftKey && !sel.selected.size) sel.click(key, "one");
          sel.click(next.dataset.rowKey!, e.shiftKey ? "range" : "one");
        }
      }
    }
  };

  return (
    <div
      role="listitem"
      tabIndex={0}
      aria-label={label}
      aria-current={active ? "true" : undefined}
      data-row-key={key}
      data-track-id={track.id}
      data-selected={picked || undefined}
      title={unavailable ? "Not available right now" : undefined}
      className={`track-row ${active ? "is-active" : ""} ${playing ? "is-playing" : ""} ${unavailable ? "is-unavailable" : ""} ${showAlbum ? "" : "track-row--no-album"} ${compact ? "track-row--compact" : ""} ${selected ? "is-selected" : ""} ${picked ? "is-picked" : ""} ${dropEdge ? `is-drop-${dropEdge}` : ""}`}
      onClick={onClick}
      onDoubleClick={(e) => {
        if ((e.target as HTMLElement).closest("button")) return;
        onPlay();
      }}
      onKeyDown={onKeyDown}
      onContextMenu={onContextMenu}
      draggable={draggable}
      onDragStart={onDragStart}
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDragEnd={onDragEnd}
      onDrop={onDrop}
    >
      {!compact && (
        <div className="track-row__num">
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
      <div className="track-row__main">
        {(showArt || compact) && (
          <span className="track-row__art">
            <Art path={track.thumb} size={40} />
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
          <div className="track-row__title truncate">{track.title}</div>
          {showArtist && (
            <div className="track-row__artist truncate">
              <button
                type="button"
                tabIndex={-1}
                className="link"
                onClick={(e) => {
                  e.stopPropagation();
                  nav.go({ name: "artist", id: track.artistId });
                }}
              >
                {trackArtistLine(track)}
              </button>
            </div>
          )}
        </div>
      </div>
      {showAlbum && !compact && (
        <div className="track-row__album truncate">
          <button
            type="button"
            tabIndex={-1}
            className="link"
            onClick={(e) => {
              e.stopPropagation();
              nav.go({ name: "album", id: track.albumId });
            }}
          >
            {track.albumTitle}
          </button>
        </div>
      )}
      {!compact && (
        <div className="track-row__badges">
          <QualityBadge track={track} />
        </div>
      )}
      <div className="track-row__end">
        {!compact && (
          <button type="button" tabIndex={-1} className="track-row__hover-btn" aria-label="Like (coming in Phase 2)" aria-disabled="true">
            <Heart size={16} aria-hidden="true" />
          </button>
        )}
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
          aria-label={`More options for ${track.title}`}
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

/** Container for TrackRows: header row, arrow-key navigation and selection scope. */
export function TrackList({
  children,
  label,
  showAlbum = true,
  header = true,
  multiMenu = (_keys: string[], ids: string[]) => multiTrackMenuItems(ids),
}: {
  children: ReactNode;
  label: string;
  showAlbum?: boolean;
  header?: boolean;
  /** Menu for a multi-selection: row keys and track ids in list order. */
  multiMenu?: (keys: string[], trackIds: string[]) => MenuItem[];
}) {
  const root = useRef<HTMLDivElement>(null);
  const buildMulti = useRef(multiMenu);
  buildMulti.current = multiMenu;
  const stableBuild = useCallback((k: string[], ids: string[]) => buildMulti.current(k, ids), []);
  const selection = useSelection(root, stableBuild);
  return (
    <SelectionContext.Provider value={selection}>
      <div className="track-list" data-tracklist ref={root}>
        {header && (
          <div className={`track-list__head ${showAlbum ? "" : "track-row--no-album"}`} aria-hidden="true">
            <span className="track-row__num">#</span>
            <span>Title</span>
            {showAlbum && <span>Album</span>}
            <span className="track-list__badges" />
            <span className="track-list__dur">Time</span>
          </div>
        )}
        <div role="list" aria-label={label}>
          {children}
        </div>
      </div>
    </SelectionContext.Provider>
  );
}

export function TrackRowSkeleton({ showArt = false, showAlbum = true }: { showArt?: boolean; showAlbum?: boolean }) {
  return (
    <div className={`track-row track-row--skeleton ${showAlbum ? "" : "track-row--no-album"}`} aria-hidden="true">
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
        <div>
          <Skeleton width="50%" height={12} />
        </div>
      )}
      <div />
      <div className="track-row__end">
        <Skeleton width={32} height={12} />
      </div>
    </div>
  );
}
