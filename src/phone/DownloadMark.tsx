/**
 * The phone's one download mark: a small arrow in a circle, filled when
 * it's on the phone, and a ring that fills while it comes. The same glyph
 * sits before a row's grey line (songs, albums, playlists) and, larger, in
 * an album or playlist header. Its state comes only from the core's songs:
 * an item is downloaded when every one of its songs is.
 */
import { ArrowGlyph, RingGlyph } from "../components/DownloadButton";
import type { DownloadItem, DownloadKind } from "../lib/types";
import { useDownloadItem, useDownloads, useIsDownloaded } from "../stores/downloads";

export type PhoneDownloadState = "none" | "waiting" | "downloading" | "done" | "error";

/**
 * Where an item stands, from its songs alone: done only when every song is
 * (a list whose songs aren't known yet is still waiting), an error once
 * every song has settled with some failed or gone.
 */
export function phoneDownloadState(item: Pick<DownloadItem, "state" | "total" | "done" | "failed" | "missing"> | undefined): PhoneDownloadState {
  if (!item) return "none";
  if (item.total > 0 && item.done >= item.total) return "done";
  if (item.total > 0 && item.done + item.failed + item.missing >= item.total && item.failed + item.missing > 0) return "error";
  if (item.state === "error" && item.total > 0) return "error";
  return item.done > 0 || item.state === "downloading" ? "downloading" : "waiting";
}

/** How far along, 0 to 1, never full before the last song is done. */
export function phoneDownloadProgress(item: Pick<DownloadItem, "total" | "done" | "progress"> | undefined): number {
  if (!item || item.total === 0) return 0;
  const whole = item.done / item.total;
  if (item.done >= item.total) return 1;
  // The song in flight counts, but it can't carry the ring to the end.
  return Math.min(Math.max(whole, item.progress || 0), (item.total - 0.5) / item.total);
}

/** "12/19" while songs are still coming. */
export const countText = (item: Pick<DownloadItem, "total" | "done">) => (item.total ? `${item.done}/${item.total}` : "");

/** "Downloaded", or "Downloading, 40%" (what the mark says to a screen reader). */
export function downloadMarkLabel(state: PhoneDownloadState, progress: number): string | null {
  if (state === "done") return "Downloaded";
  if (state === "downloading" || state === "waiting") return `Downloading, ${Math.round(Math.max(0, Math.min(1, progress)) * 100)}%`;
  return null;
}

/** The glyph alone, for a state. Nothing for an item that isn't downloading. */
export function DownloadGlyph({ state, progress = 0, size }: { state: PhoneDownloadState; progress?: number; size: number }) {
  if (state === "done") return <ArrowGlyph size={size} filled />;
  if (state === "downloading" || state === "waiting") return <RingGlyph size={size} progress={progress} waiting={state === "waiting" && progress <= 0} />;
  return null;
}

/** The small mark before a row's grey line, labelled for screen readers. */
export function DownloadMark({ state, progress = 0, size = 14 }: { state: PhoneDownloadState; progress?: number; size?: number }) {
  const label = downloadMarkLabel(state, progress);
  if (!label) return null;
  return (
    <span className={`ph-glyph ph-glyph--dl is-${state}`} role="img" aria-label={label} data-testid="download-mark">
      <DownloadGlyph state={state} progress={progress} size={size} />
    </span>
  );
}

/** An album's, playlist's or Liked Songs' mark, from its download; `downloaded` covers what the core hasn't listed yet. */
export function ItemDownloadMark({ kind, id, downloaded }: { kind: DownloadKind; id: string; downloaded?: boolean }) {
  const item = useDownloadItem(kind, id);
  const state = phoneDownloadState(item);
  if (state === "none" || state === "error") return downloaded ? <DownloadMark state="done" /> : null;
  return <DownloadMark state={state} progress={phoneDownloadProgress(item)} />;
}

/** A song's mark: filled once it's on the phone, a ring while it's the one coming down. */
export function TrackDownloadMark({ trackId }: { trackId: string }) {
  const done = useIsDownloaded(trackId);
  const coming = useDownloads((s) => {
    const c = s.status?.current;
    if (!c || c.trackId !== trackId) return null;
    return c.total ? c.received / c.total : 0;
  });
  if (done) return <DownloadMark state="done" />;
  if (coming !== null) return <DownloadMark state="downloading" progress={coming} />;
  return null;
}
