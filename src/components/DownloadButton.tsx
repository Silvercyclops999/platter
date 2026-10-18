/**
 * The download control for album, playlist, Liked Songs and mix pages, the
 * mark on downloaded rows, the remove confirmation and the storage banner.
 *
 * A page drops in `<DownloadButton kind id title />` beside its other
 * action-row buttons (a mix also passes `trackIds`). States: not downloaded
 * (an outlined arrow), waiting, downloading (a ring that fills, with the
 * percent beside it), downloaded (a filled disc) and error (some songs
 * couldn't download or went missing; a click tries them again).
 */
import { AlertCircle } from "lucide-react";
import { useRef } from "react";
import { formatBytes } from "../lib/format";
import type { DownloadKind } from "../lib/types";
import {
  answerRemove,
  itemText,
  retryDownload,
  toggleDownload,
  useDownloadItem,
  useDownloads,
  useIsDownloaded,
} from "../stores/downloads";
import { nav } from "../stores/router";
import { Banner } from "./Banner";
import { Button, IconButton } from "./Button";
import { Dialog } from "./Dialog";
import "./downloads.css";

/** A down arrow inside a circle; `filled` is the downloaded disc. */
function ArrowGlyph({ size, filled }: { size: number; filled?: boolean }) {
  return (
    <svg viewBox="0 0 24 24" width={size} height={size} aria-hidden="true" className={`dl-glyph ${filled ? "dl-glyph--filled" : ""}`}>
      {filled ? (
        <circle cx="12" cy="12" r="11" className="dl-glyph__disc" />
      ) : (
        <circle cx="12" cy="12" r="10.25" fill="none" stroke="currentColor" strokeWidth="1.5" />
      )}
      <path
        d="M12 6.75v9M8.25 12.5L12 16.25l3.75-3.75"
        fill="none"
        className={filled ? "dl-glyph__arrow-on" : undefined}
        stroke={filled ? undefined : "currentColor"}
        strokeWidth="1.75"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

/** A progress ring (0 to 1) around a small arrow. */
function RingGlyph({ size, progress, waiting }: { size: number; progress: number; waiting?: boolean }) {
  const r = 10.25;
  const c = 2 * Math.PI * r;
  return (
    <svg viewBox="0 0 24 24" width={size} height={size} aria-hidden="true" className={`dl-ring ${waiting ? "dl-ring--waiting" : ""}`}>
      <circle cx="12" cy="12" r={r} className="dl-ring__track" />
      {!waiting && (
        <circle
          cx="12"
          cy="12"
          r={r}
          className="dl-ring__fill"
          strokeDasharray={c}
          strokeDashoffset={c * (1 - Math.max(0.02, Math.min(1, progress)))}
          transform="rotate(-90 12 12)"
        />
      )}
      <path d="M12 8.5v6.5M9.5 12.5L12 15l2.5-2.5" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

export interface DownloadButtonProps {
  kind: DownloadKind;
  id: string;
  title: string;
  /** A mix's songs, read when the button is clicked. */
  trackIds?: () => string[];
  disabled?: boolean;
  size?: "md" | "lg";
}

export function DownloadButton({ kind, id, title, trackIds, disabled, size = "lg" }: DownloadButtonProps) {
  const item = useDownloadItem(kind, id);
  const paused = useDownloads((s) => s.status?.paused ?? null);
  const px = size === "lg" ? 28 : 22;
  const target = { kind, id, title, trackIds };
  const text = itemText(item, paused);
  const pct = item?.state === "downloading" ? Math.round(item.progress * 100) : null;

  let icon;
  let label: string;
  let onClick = () => toggleDownload(target);
  let tooltip: string = text;
  switch (item?.state) {
    case undefined:
      icon = <ArrowGlyph size={px} />;
      label = `Download ${title}`;
      tooltip = "Download";
      break;
    case "downloaded":
      icon = <ArrowGlyph size={px} filled />;
      label = `Remove download of ${title}`;
      tooltip = "Remove download";
      break;
    case "error":
      icon = <AlertCircle size={px - 2} strokeWidth={1.75} />;
      label = `${text}. Download ${title} again`;
      tooltip = `${item.error ?? text}. Click to try again.`;
      onClick = () => void retryDownload(kind, id);
      break;
    default:
      icon = <RingGlyph size={px} progress={item!.progress} waiting={item!.state === "queued"} />;
      label = `${text}. Stop downloading ${title}`;
      tooltip = `${text}. Click to stop.`;
  }

  return (
    <span className={`dl-btn dl-btn--${item?.state ?? "none"}`} data-testid="download-button">
      <IconButton
        label={label}
        tooltip={tooltip}
        size={size}
        pressed={item?.state === "downloaded"}
        disabled={disabled}
        icon={icon}
        onClick={onClick}
        className="dl-btn__button"
      />
      {pct !== null && (
        <span className="dl-btn__pct tabular" aria-hidden="true">
          {pct}%
        </span>
      )}
    </span>
  );
}

/** The small mark on a downloaded song's row. */
export function DownloadedMark({ trackId }: { trackId: string }) {
  const on = useIsDownloaded(trackId);
  if (!on) return null;
  return (
    <span className="dl-mark" role="img" aria-label="Downloaded" title="Downloaded">
      <ArrowGlyph size={14} filled />
    </span>
  );
}

/** The small mark on a downloaded library item (before its subtitle). */
export function DownloadedGlyph() {
  return (
    <span className="dl-mark dl-mark--lib" role="img" aria-label="Downloaded">
      <ArrowGlyph size={12} filled />
    </span>
  );
}

/** Asks before removing a download. Mounted once in the shell. */
export function DownloadDialogs() {
  const ask = useDownloads((s) => s.confirming);
  const cancel = useRef<HTMLButtonElement>(null);
  if (!ask) return null;
  return (
    <Dialog open onClose={() => answerRemove(false)} label="Remove from downloads?" className="confirm" initialFocus={cancel}>
      <h2 className="confirm__title">Remove from downloads?</h2>
      <p className="confirm__body">
        You won't be able to play this offline.
        {ask.bytes > 0 && ` It frees ${formatBytes(ask.bytes)} on this computer.`}
      </p>
      <div className="confirm__actions">
        <Button ref={cancel} variant="ghost" onClick={() => answerRemove(false)}>
          Cancel
        </Button>
        <Button variant="primary" onClick={() => answerRemove(true)}>
          Remove
        </Button>
      </div>
    </Dialog>
  );
}

/** Shown while downloads are stopped by the storage limit or a full disk. */
export function DownloadsBanner() {
  const paused = useDownloads((s) => s.status?.paused ?? null);
  const waiting = useDownloads((s) => (s.status?.queued ?? 0) > 0);
  if (!waiting || (paused !== "full" && paused !== "noSpace")) return null;
  const msg =
    paused === "full"
      ? "Downloads stopped: they reached the storage limit you set. Remove some downloads or raise the limit in Settings."
      : "Downloads stopped: this computer is out of space. Free some space or move downloads to another drive in Settings.";
  return (
    <div className="main__banner" data-testid="downloads-banner">
      <Banner
        kind="warning"
        action={
          <Button size="sm" variant="secondary" onClick={() => nav.go({ name: "settings" })}>
            Open Settings
          </Button>
        }
      >
        {msg}
      </Banner>
    </div>
  );
}

