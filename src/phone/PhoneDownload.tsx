/**
 * The download control in an album, playlist, Liked Songs or mix header:
 * the same mark the rows show (DownloadMark), larger, and a tap to start,
 * stop or remove it. A ring fills while the songs come; there's no count
 * beside it.
 */
import { AlertCircle } from "lucide-react";
import { ArrowGlyph } from "../components/DownloadButton";
import type { DownloadKind } from "../lib/types";
import { retryDownload, toggleDownload, useDownloadItem, useDownloads } from "../stores/downloads";
import { tap } from "./bits";
import { DownloadGlyph, downloadMarkLabel, phoneDownloadProgress, phoneDownloadState } from "./DownloadMark";

export { countText, phoneDownloadProgress, phoneDownloadState, type PhoneDownloadState } from "./DownloadMark";

export function PhoneDownload({ kind, id, title, trackIds, size = 24 }: { kind: DownloadKind; id: string; title: string; trackIds?: () => string[]; size?: number }) {
  const item = useDownloadItem(kind, id);
  const paused = useDownloads((s) => s.status?.paused ?? null);
  const state = phoneDownloadState(item);
  const progress = phoneDownloadProgress(item);
  const target = { kind, id, title, trackIds };
  const press = () => {
    tap();
    if (state === "error") void retryDownload(kind, id);
    else toggleDownload(target, { askAlways: true });
  };
  const label =
    state === "none"
      ? `Download ${title}`
      : state === "done"
        ? `Remove download of ${title}`
        : state === "error"
          ? `Some songs of ${title} didn't download. Try again`
          : state === "waiting" && !item?.done
            ? `${paused ? "Waiting" : "Waiting to download"}. Stop downloading ${title}`
            : `${downloadMarkLabel(state, progress)}. Stop downloading ${title}`;
  return (
    <button type="button" className={`ph-icon-btn ph-dl is-${state}`} aria-label={label} aria-pressed={state === "done"} onClick={press} data-testid="phone-download">
      {state === "none" && <ArrowGlyph size={size} />}
      {state === "error" && <AlertCircle size={size - 2} strokeWidth={1.75} aria-hidden="true" />}
      <DownloadGlyph state={state} progress={progress} size={size} />
    </button>
  );
}
