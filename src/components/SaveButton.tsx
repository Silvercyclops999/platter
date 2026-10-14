import type { MouseEvent } from "react";
import type { Track } from "../lib/types";
import { setLiked, useIsLiked, useLikes } from "../stores/likes";
import { LIKE_SHORTCUT } from "../shell/useShortcuts";
import { useSettings } from "../stores/settings";
import { Tooltip } from "./Tooltip";
import "./likes.css";

/** The not-liked glyph: an outlined circle with a plus. */
function PlusGlyph() {
  return (
    <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true" className="save-btn__glyph">
      <circle cx="8" cy="8" r="7" fill="none" stroke="currentColor" strokeWidth="1.5" />
      <path d="M8 4.75v6.5M4.75 8h6.5" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
    </svg>
  );
}

/** The liked glyph: a filled accent circle with a tick. */
function TickGlyph() {
  return (
    <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true" className="save-btn__glyph save-btn__glyph--tick">
      <circle cx="8" cy="8" r="8" className="save-btn__disc" />
      <path d="M4.7 8.2l2.2 2.2 4.4-4.6" fill="none" className="save-btn__tick" strokeWidth="1.75" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

export interface SaveButtonProps {
  track: Track;
  /** Where it sits: track rows hide it until hover (Settings); the player bar and now-playing panel always show it. */
  place: "row" | "bar" | "panel";
  /** Rows of the Liked Songs page: the tick shows on hover only (every row is liked there). */
  quietWhenLiked?: boolean;
}

/**
 * The like control, the same in rows, the player bar and the now-playing
 * panel. Not liked: an outlined plus; one click likes the song. Liked: a
 * filled tick; one click opens the "saved in" menu (where it can be unliked
 * or added to playlists).
 */
export function SaveButton({ track, place, quietWhenLiked }: SaveButtonProps) {
  const liked = useIsLiked(track.id);
  const rows = useSettings((s) => s.settings?.likeControlRows ?? "hover");
  if (place === "row" && rows === "off") return null;
  const label = liked ? "Add to playlist" : "Add to Liked Songs";
  // The bar and the panel hold the playing song, which the shortcut likes.
  const tip = place === "row" ? label : `${label} (${liked ? "unlike: " : ""}${LIKE_SHORTCUT})`;
  const onClick = (e: MouseEvent<HTMLButtonElement>) => {
    e.stopPropagation();
    const el = e.currentTarget;
    if (liked) useLikes.getState().openSavedIn(track, el);
    else void setLiked([track], true, { from: el });
  };
  return (
    <Tooltip label={tip}>
      <button
        type="button"
        tabIndex={place === "row" ? -1 : undefined}
        className={`save-btn save-btn--${place} ${liked ? "is-liked" : ""} ${quietWhenLiked ? "is-quiet" : ""} ${place === "row" && rows === "always" ? "is-always" : ""}`}
        aria-label={liked ? `${track.title} is in Liked Songs. Add to playlist` : `Add ${track.title} to Liked Songs`}
        aria-haspopup={liked ? "dialog" : undefined}
        aria-pressed={liked}
        data-liked={liked || undefined}
        onClick={onClick}
        onDoubleClick={(e) => e.stopPropagation()}
      >
        {liked ? <TickGlyph /> : <PlusGlyph />}
      </button>
    </Tooltip>
  );
}
