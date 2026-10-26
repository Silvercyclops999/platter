/**
 * The phone's download marks. Before a row's grey line (songs, albums,
 * playlists, mixes): a hollow arrow while it waits, a small spinner while it
 * comes, a filled arrow once it's on the phone, nothing otherwise. In a
 * header, the button: an outlined arrow, a thin ring with a turning arc while
 * it waits, a stop square in a filling ring while songs come, a filled disc
 * when done. Its state comes only from the core's songs: an item is
 * downloaded when every one of its songs is.
 */
import { createContext, useContext, type ReactNode } from "react";
import { ArrowGlyph } from "../components/DownloadButton";
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
  // Partly here but held (offline, away, full) or behind another list: waiting, not coming.
  return item.state === "downloading" ? "downloading" : "waiting";
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

/** "Downloaded", "Waiting to download" or "Downloading, 40%" (what the mark says to a screen reader). */
export function downloadMarkLabel(state: PhoneDownloadState, progress: number): string | null {
  if (state === "done") return "Downloaded";
  if (state === "waiting") return "Waiting to download";
  if (state === "downloading") return `Downloading, ${Math.round(Math.max(0, Math.min(1, progress)) * 100)}%`;
  return null;
}

const R = 10.25;
const C = 2 * Math.PI * R;

/** A thin ring with a short arc turning on it: waiting its turn. */
export function WaitGlyph({ size }: { size: number }) {
  return (
    <svg viewBox="0 0 24 24" width={size} height={size} aria-hidden="true" className="ph-dl-wait">
      <circle cx="12" cy="12" r={R} className="ph-dl-wait__track" />
      <circle cx="12" cy="12" r={R} className="ph-dl-wait__arc" strokeDasharray={`${C * 0.22} ${C}`} />
    </svg>
  );
}

/** A stop square in a ring that fills as the songs come (0 to 1). */
export function StopRingGlyph({ size, progress }: { size: number; progress: number }) {
  return (
    <svg viewBox="0 0 24 24" width={size} height={size} aria-hidden="true" className="ph-dl-stop">
      <circle cx="12" cy="12" r={R} className="ph-dl-stop__track" />
      <circle
        cx="12"
        cy="12"
        r={R}
        className="ph-dl-stop__fill"
        strokeDasharray={C}
        strokeDashoffset={C * (1 - Math.max(0.02, Math.min(1, progress)))}
        transform="rotate(-90 12 12)"
      />
      <rect x="8.75" y="8.75" width="6.5" height="6.5" rx="1" className="ph-dl-stop__square" />
    </svg>
  );
}

/** A small turning arc: this one is coming down now. */
export function SpinGlyph({ size }: { size: number }) {
  return (
    <svg viewBox="0 0 24 24" width={size} height={size} aria-hidden="true" className="ph-dl-spin">
      <circle cx="12" cy="12" r="9" className="ph-dl-spin__arc" strokeDasharray={`${2 * Math.PI * 9 * 0.7} ${2 * Math.PI * 9}`} />
    </svg>
  );
}

/** A header button's glyph, for a state. Nothing for one that isn't downloading. */
export function DownloadGlyph({ state, progress = 0, size }: { state: PhoneDownloadState; progress?: number; size: number }) {
  if (state === "done") return <ArrowGlyph size={size} filled />;
  if (state === "waiting") return <WaitGlyph size={size} />;
  if (state === "downloading") return <StopRingGlyph size={size} progress={progress} />;
  return null;
}

/** A row's glyph: a hollow arrow waiting, a spinner coming, a filled arrow here. */
export function RowDownloadGlyph({ state, size }: { state: PhoneDownloadState; size: number }) {
  if (state === "done") return <ArrowGlyph size={size} filled />;
  if (state === "waiting") return <ArrowGlyph size={size} />;
  if (state === "downloading") return <SpinGlyph size={size} />;
  return null;
}

/** The small mark before a row's grey line, labelled for screen readers. */
export function DownloadMark({ state, progress = 0, size = 14 }: { state: PhoneDownloadState; progress?: number; size?: number }) {
  const label = downloadMarkLabel(state, progress);
  if (!label) return null;
  return (
    <span className={`ph-glyph ph-glyph--dl is-${state}`} role="img" aria-label={label} data-testid="download-mark">
      <RowDownloadGlyph state={state} size={size} />
    </span>
  );
}

/**
 * Set by a list's page while its download is under way: its songs not on
 * the phone yet show the waiting arrow until their turn.
 */
export const ListDownloading = createContext(false);

/** An album's, playlist's or Liked Songs' mark, from its download; `downloaded` covers what the core hasn't listed yet. */
export function ItemDownloadMark({ kind, id, downloaded }: { kind: DownloadKind; id: string; downloaded?: boolean }) {
  const item = useDownloadItem(kind, id);
  const state = phoneDownloadState(item);
  if (state === "none" || state === "error") return downloaded ? <DownloadMark state="done" /> : null;
  return <DownloadMark state={state} progress={phoneDownloadProgress(item)} />;
}

/**
 * A song's mark: filled once it's on the phone, a spinner while it's the one
 * coming down, the waiting arrow while its list is downloading.
 */
export function TrackDownloadMark({ trackId }: { trackId: string }) {
  const done = useIsDownloaded(trackId);
  const listOn = useContext(ListDownloading);
  const coming = useDownloads((s) => {
    const c = s.status?.current;
    if (!c || c.trackId !== trackId) return null;
    return c.total ? c.received / c.total : 0;
  });
  if (done) return <DownloadMark state="done" />;
  if (coming !== null) return <DownloadMark state="downloading" progress={coming} />;
  if (listOn) return <DownloadMark state="waiting" />;
  return null;
}

/** Whether a list's download is under way (waiting or coming), for `ListDownloading`. */
export function useListDownloading(kind: DownloadKind, id: string): boolean {
  const s = phoneDownloadState(useDownloadItem(kind, id));
  return s === "waiting" || s === "downloading";
}

/** Wraps a list's songs so those not on the phone yet show the waiting arrow while its download is under way. */
export function ListDownloadingFor({ item, children }: { item?: { kind: DownloadKind; id: string }; children: ReactNode }) {
  return item ? <ListOf kind={item.kind} id={item.id}>{children}</ListOf> : <>{children}</>;
}

function ListOf({ kind, id, children }: { kind: DownloadKind; id: string; children: ReactNode }) {
  return <ListDownloading.Provider value={useListDownloading(kind, id)}>{children}</ListDownloading.Provider>;
}
