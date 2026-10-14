import { Fragment, useMemo, type KeyboardEvent, type MouseEvent, type ReactNode } from "react";
import { trackCredits, type Credit } from "../lib/credits";
import type { Track } from "../lib/types";
import { useApp } from "../stores/app";
import { nav } from "../stores/router";

type CreditTrack = Pick<Track, "trackArtist" | "artistTitle" | "artistId">;

/** A track's credits, split against the library's artist names. */
export function useTrackCredits(track: CreditTrack): Credit[] {
  const lookup = useApp((s) => s.artistLookup);
  const { trackArtist, artistTitle, artistId } = track;
  return useMemo(() => trackCredits({ trackArtist, artistTitle, artistId }, lookup), [trackArtist, artistTitle, artistId, lookup]);
}

/**
 * A track's artist line with every library artist as its own link. Links are
 * inline text (not buttons) so the whole line ellipsizes as one when it runs
 * out of room. Clicks and keys stay on the link, so a row around it neither
 * plays nor selects.
 */
export function ArtistLinks({ track, className = "" }: { track: CreditTrack; className?: string }) {
  const credits = useTrackCredits(track);
  return (
    <span className={`artist-line ${className}`}>
      {credits.map((c, i) => (
        <Fragment key={`${c.name}-${i}`}>
          {i > 0 && ", "}
          {c.id ? <TextLink onActivate={() => nav.go({ name: "artist", id: c.id! })}>{c.name}</TextLink> : c.name}
        </Fragment>
      ))}
    </span>
  );
}

/**
 * Inline text link (artist or album name inside a line of text). A span, not
 * a button: buttons are atomic boxes that get cut off hard instead of ending
 * in an ellipsis.
 */
export function TextLink({
  onActivate,
  children,
  className = "",
  tabIndex = 0,
}: {
  onActivate: () => void;
  children: ReactNode;
  className?: string;
  tabIndex?: number;
}) {
  return (
    <span
      role="link"
      tabIndex={tabIndex}
      className={`link text-link ${className}`}
      onClick={(e: MouseEvent) => {
        e.stopPropagation();
        onActivate();
      }}
      onDoubleClick={(e: MouseEvent) => e.stopPropagation()}
      onKeyDown={(e: KeyboardEvent) => {
        if (e.key !== "Enter") return;
        e.preventDefault();
        e.stopPropagation();
        onActivate();
      }}
    >
      {children}
    </span>
  );
}
