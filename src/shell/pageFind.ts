/**
 * Ctrl/Cmd+F on a page with its own song finder (a playlist, Liked Songs,
 * a top songs list) opens that finder; everywhere else it filters Your
 * Library. The page on screen registers its opener while it's mounted, and
 * how to bring a song's row into view for "back to the playing song".
 */
import { useEffect } from "react";

let opener: (() => void) | null = null;

/** Register the page's finder while this page is shown. `open` should be stable. */
export function usePageFind(open: (() => void) | null) {
  useEffect(() => {
    if (!open) return;
    opener = open;
    return () => {
      if (opener === open) opener = null;
    };
  }, [open]);
}

/** Open the page's finder; false when the page has none. */
export function openPageFind(): boolean {
  if (!opener) return false;
  opener();
  return true;
}

let revealer: ((trackId: string) => void) | null = null;

/**
 * Register how the page brings a song's row into the DOM for "back to the
 * playing song": scroll a virtual list to it, or clear a finder hiding it.
 * `reveal` should be stable.
 */
export function usePageReveal(reveal: ((trackId: string) => void) | null) {
  useEffect(() => {
    if (!reveal) return;
    revealer = reveal;
    return () => {
      if (revealer === reveal) revealer = null;
    };
  }, [reveal]);
}

/** Ask the page to bring the row for `trackId` onto the page. */
export function pageReveal(trackId: string) {
  revealer?.(trackId);
}
