/**
 * Downloads on the phone: the one download mark (the same arrow in a circle
 * on rows, headers and the library), and what an album, playlist, Liked
 * Songs or mix says while it comes onto the phone. Its state comes only from
 * the core's songs: it's downloaded when every one of its songs is, and a
 * ring with "12/19" shows until then.
 */
import { AlertCircle } from "lucide-react";
import { ArrowGlyph, RingGlyph } from "../components/DownloadButton";
import type { DownloadItem, DownloadKind } from "../lib/types";
import { retryDownload, toggleDownload, useDownloadItem, useDownloads } from "../stores/downloads";
import { tap } from "./bits";

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

export function PhoneDownload({ kind, id, title, trackIds, size = 24 }: { kind: DownloadKind; id: string; title: string; trackIds?: () => string[]; size?: number }) {
  const item = useDownloadItem(kind, id);
  const paused = useDownloads((s) => s.status?.paused ?? null);
  const state = phoneDownloadState(item);
  const target = { kind, id, title, trackIds };
  const press = () => {
    tap();
    if (state === "error") void retryDownload(kind, id);
    else toggleDownload(target);
  };
  const count = item ? countText(item) : "";
  const label =
    state === "none"
      ? `Download ${title}`
      : state === "done"
        ? `Remove download of ${title}`
        : state === "error"
          ? `Some songs of ${title} didn't download. Try again`
          : `${state === "waiting" ? (paused ? "Waiting" : "Waiting to download") : "Downloading"}${count ? `, ${count} songs` : ""}. Stop downloading ${title}`;
  return (
    <button type="button" className={`ph-icon-btn ph-dl is-${state}`} aria-label={label} aria-pressed={state === "done"} onClick={press} data-testid="phone-download">
      {state === "none" && <ArrowGlyph size={size} />}
      {state === "done" && <ArrowGlyph size={size} filled />}
      {state === "error" && <AlertCircle size={size - 2} strokeWidth={1.75} aria-hidden="true" />}
      {(state === "waiting" || state === "downloading") && <RingGlyph size={size} progress={phoneDownloadProgress(item)} waiting={state === "waiting" && !item?.done} />}
      {(state === "waiting" || state === "downloading") && count && (
        <span className="ph-dl__count tabular" aria-hidden="true">
          {count}
        </span>
      )}
    </button>
  );
}
