import { Ellipsis, Heart, Play } from "lucide-react";
import type { KeyboardEvent, MouseEvent, ReactNode } from "react";
import { formatDuration, trackArtistLine } from "../lib/format";
import type { Track } from "../lib/types";
import { nav } from "../stores/router";
import { player } from "../stores/player";
import { toast } from "../stores/toasts";
import type { MenuItem } from "../stores/ui";
import { Art } from "./Art";
import { QualityBadge } from "./Badge";
import { openContextMenu } from "./ContextMenu";
import { Skeleton } from "./Skeleton";

/** Standard track context menu. Pages can prepend/append items. */
export function trackMenuItems(track: Track, extra: { before?: MenuItem[]; after?: MenuItem[] } = {}): MenuItem[] {
  return [
    ...(extra.before ?? []),
    { label: "Add to queue", onSelect: () => player.queueAdd([track.id], "end") },
    { label: "Go to artist", separator: true, onSelect: () => nav.go({ name: "artist", id: track.artistId }) },
    { label: "Go to album", onSelect: () => nav.go({ name: "album", id: track.albumId }) },
    {
      label: "Copy song title",
      separator: true,
      onSelect: () => {
        navigator.clipboard?.writeText(`${track.title} — ${trackArtistLine(track)}`).then(
          () => toast.info("Copied to clipboard"),
          () => toast.error("Couldn't copy"),
        );
      },
    },
    ...(extra.after ?? []),
  ];
}

export interface TrackRowProps {
  track: Track;
  /** Number shown in the first column (track number or list position). */
  index: number;
  onPlay: () => void;
  /** This row is the current track. */
  active?: boolean;
  /** …and it is playing (shows the equalizer glyph). */
  playing?: boolean;
  showArt?: boolean;
  showAlbum?: boolean;
  /** Show the artist line under the title (off on single-artist albums when equal). */
  showArtist?: boolean;
  menuItems?: MenuItem[];
  /** Extra trailing content (e.g. remove button in the queue). */
  trailing?: ReactNode;
  compact?: boolean;
  /** Highlighted by keyboard navigation that lives elsewhere (search box arrows). */
  selected?: boolean;
  draggable?: boolean;
  onDragStart?: (e: React.DragEvent) => void;
  onDragOver?: (e: React.DragEvent) => void;
  onDrop?: (e: React.DragEvent) => void;
}

/**
 * One track line: number ⇄ play on hover, art, title + artist, album,
 * badges, duration and hover actions. Enter plays; Shift+F10 / Menu key opens
 * the context menu; right-click too.
 */
export function TrackRow({
  track,
  index,
  onPlay,
  active,
  playing,
  showArt = false,
  showAlbum = true,
  showArtist = true,
  menuItems,
  trailing,
  compact,
  selected,
  draggable,
  onDragStart,
  onDragOver,
  onDrop,
}: TrackRowProps) {
  const items = menuItems ?? trackMenuItems(track);
  const label = `${track.title} by ${trackArtistLine(track)}`;

  const onContextMenu = (e: MouseEvent<HTMLDivElement>) => {
    e.preventDefault();
    openContextMenu(`Options for ${track.title}`, items, e);
  };
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.target !== e.currentTarget) return;
    if (e.key === "Enter") {
      e.preventDefault();
      onPlay();
    } else if (e.key === "ContextMenu" || (e.key === "F10" && e.shiftKey)) {
      e.preventDefault();
      openContextMenu(`Options for ${track.title}`, items, e.currentTarget);
    } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      // Move between rows in the same list.
      const list = e.currentTarget.closest('[data-tracklist]');
      const rows = Array.from(list?.querySelectorAll<HTMLElement>(".track-row") ?? []);
      const i = rows.indexOf(e.currentTarget);
      const next = rows[i + (e.key === "ArrowDown" ? 1 : -1)];
      if (next) {
        e.preventDefault();
        next.focus();
      }
    }
  };

  return (
    <div
      role="listitem"
      tabIndex={0}
      aria-label={label}
      aria-current={active ? "true" : undefined}
      className={`track-row ${active ? "is-active" : ""} ${showAlbum ? "" : "track-row--no-album"} ${compact ? "track-row--compact" : ""} ${selected ? "is-selected" : ""}`}
      onDoubleClick={onPlay}
      onKeyDown={onKeyDown}
      onContextMenu={onContextMenu}
      draggable={draggable}
      onDragStart={onDragStart}
      onDragOver={onDragOver}
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
            aria-label={`Play ${track.title}`}
            onClick={(e) => {
              e.stopPropagation();
              onPlay();
            }}
          >
            <Play size={14} fill="currentColor" aria-hidden="true" />
          </button>
        </div>
      )}
      <div className="track-row__main">
        {(showArt || compact) && (
          <span className="track-row__art">
            <Art path={track.thumb} size={compact ? 40 : 40} />
            {compact && (
              <button
                type="button"
                tabIndex={-1}
                className="track-row__art-play"
                aria-label={`Play ${track.title}`}
                onClick={(e) => {
                  e.stopPropagation();
                  onPlay();
                }}
              >
                <Play size={14} fill="currentColor" aria-hidden="true" />
              </button>
            )}
          </span>
        )}
        <div className="track-row__text">
          <div className="track-row__title truncate">
            {track.title}
          </div>
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
        <button
          type="button"
          tabIndex={-1}
          className="track-row__hover-btn"
          aria-label={`More options for ${track.title}`}
          onClick={(e) => {
            e.stopPropagation();
            openContextMenu(`Options for ${track.title}`, items, e.currentTarget);
          }}
        >
          <Ellipsis size={16} aria-hidden="true" />
        </button>
      </div>
    </div>
  );
}

/** Container for TrackRows: header row + arrow-key navigation scope. */
export function TrackList({
  children,
  label,
  showAlbum = true,
  header = true,
}: {
  children: ReactNode;
  label: string;
  showAlbum?: boolean;
  header?: boolean;
}) {
  return (
    <div className="track-list" data-tracklist>
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
