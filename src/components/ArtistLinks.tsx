import { Fragment, useMemo, type KeyboardEvent, type MouseEvent, type ReactNode } from "react";
import { trackCredits, type Credit } from "../lib/credits";
import type { Track } from "../lib/types";
import { useApp } from "../stores/app";
import { nav } from "../stores/router";
import { useSettings } from "../stores/settings";

/** The "Link featured artists" setting (on until settings load). */
export const useLinkFeatured = (): boolean => useSettings((s) => s.settings?.linkFeatured ?? true);
export const linkFeaturedNow = (): boolean => useSettings.getState().settings?.linkFeatured ?? true;

type CreditTrack = Pick<Track, "trackArtist" | "artistTitle" | "artistId">;

/** A track's credits, split against the library's artist names. */
export function useTrackCredits(track: CreditTrack): Credit[] {
  const lookup = useApp((s) => s.artistLookup);
  const linkAll = useLinkFeatured();
  const { trackArtist, artistTitle, artistId } = track;
  return useMemo(
    () => trackCredits({ trackArtist, artistTitle, artistId }, lookup, linkAll),
    [trackArtist, artistTitle, artistId, lookup, linkAll],
  );
}

/**
 * A track's artist line with every credited artist as its own link (library
 * artists, and featured names with a generated page). Links are
 * inline text (not buttons) so the whole line ellipsizes as one when it runs
 * out of room. Clicks and keys stay on the link, so a row around it neither
 * plays nor selects. Inside a track row the links aren't Tab stops
 * (`linkTabIndex` -1); arrows reach them from the row.
 */
export function ArtistLinks({ track, className = "", linkTabIndex = 0 }: { track: CreditTrack; className?: string; linkTabIndex?: number }) {
  const credits = useTrackCredits(track);
  return (
    <span className={`artist-line ${className}`}>
      {credits.map((c, i) => (
        <Fragment key={`${c.name}-${i}`}>
          {i > 0 && ", "}
          {c.id ? (
            <TextLink tabIndex={linkTabIndex} onActivate={() => nav.go({ name: "artist", id: c.id! })}>
              {c.name}
            </TextLink>
          ) : (
            c.name
          )}
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
