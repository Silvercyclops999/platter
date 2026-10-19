/**
 * Small pieces the phone screens share: the format plate, the bottom sheet
 * frame, row glyphs, a like control and a light tap of feedback.
 */
import { Pin } from "lucide-react";
import { useEffect, useId, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { Plate } from "../components/Plate";
import { prefersReducedMotion } from "../design/motion";
import { usePlayer } from "../stores/player";
import { DownloadedGlyph } from "../components/DownloadButton";
import { PlusGlyph, TickGlyph } from "../components/SaveButton";
import { qualityLabel } from "../lib/format";
import type { Track } from "../lib/types";
import { useIsDownloaded } from "../stores/downloads";
import { setLiked, useIsLiked } from "../stores/likes";
import { useSettings } from "../stores/settings";
import { phone } from "./phoneUi";

/** A light tap of feedback on a control, while Haptics on controls is on. */
export function tap() {
  if (useSettings.getState().settings?.phoneHaptics === false) return;
  try {
    if (typeof navigator !== "undefined" && "vibrate" in navigator) navigator.vibrate?.(8);
  } catch {
    // No vibration on this device.
  }
}

/** What the plate says: "Downloaded", "FLAC 24/96", a bitrate, or nothing known. */
export function plateText(track: Pick<Track, "codec" | "bitDepth" | "sampleRate" | "bitrate">, downloaded: boolean): string | null {
  return downloaded ? "Downloaded" : qualityLabel(track);
}

/** The engraved format plate beside an artist line. */
export function FormatPlate({ track, className = "" }: { track: Track; className?: string }) {
  const downloaded = useIsDownloaded(track.id);
  const text = plateText(track, downloaded);
  if (!text) return null;
  const hiRes = !downloaded && ((track.bitDepth ?? 0) > 16 || (track.sampleRate ?? 0) > 48_000);
  return (
    <Plate size="sm" tone={hiRes ? "lit" : "plain"} className={`ph-plate ${className}`}>
      {text}
    </Plate>
  );
}

/** Glyphs before a library row's grey line: pinned, downloaded. */
export function RowGlyphs({ pinned, downloaded }: { pinned?: boolean; downloaded?: boolean }) {
  if (!pinned && !downloaded) return null;
  return (
    <span className="ph-glyphs">
      {pinned && (
        <span className="ph-glyph ph-glyph--pin" role="img" aria-label="Pinned">
          <Pin size={12} aria-hidden="true" />
        </span>
      )}
      {downloaded && <DownloadedGlyph />}
    </span>
  );
}

/** Like a song (plus), or, once liked, open its sheet (tick) to unlike or add it to a playlist. */
export function LikeControl({ track, size = 24 }: { track: Track; size?: number }) {
  const liked = useIsLiked(track.id);
  return (
    <button
      type="button"
      className={`ph-icon-btn ph-like ${liked ? "is-on" : ""}`}
      style={{ width: size + 20, height: size + 20 }}
      aria-label={liked ? "Liked. More for this song" : "Add to Liked Songs"}
      aria-pressed={liked}
      onClick={() => {
        tap();
        if (liked) phone.push({ kind: "track", track });
        else void setLiked([track], true);
      }}
    >
      {liked ? <TickGlyph /> : <PlusGlyph />}
    </button>
  );
}

/**
 * A bottom sheet: a handle, a heading, and its body over the dimmed screen.
 * Tapping the dim area or Escape closes it (back to the sheet under it).
 */
export function SheetFrame({
  title,
  subtitle,
  onClose,
  children,
  footer,
  tall = false,
  head,
  action,
  className = "",
}: {
  title: string;
  subtitle?: ReactNode;
  onClose: () => void;
  children: ReactNode;
  footer?: ReactNode;
  tall?: boolean;
  /** Replaces the plain heading (a track's art and name). */
  head?: ReactNode;
  /** A control at the right of the heading (the queue's Edit). */
  action?: ReactNode;
  className?: string;
}) {
  const id = useId();
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    box.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
  return (
    <div className="ph-sheet-layer">
      <div className="ph-sheet-dim" data-testid="sheet-dim" onClick={onClose} aria-hidden="true" />
      <div
        ref={box}
        className={`ph-sheet ${tall ? "ph-sheet--tall" : ""} ${className}`}
        role="dialog"
        aria-modal="true"
        aria-labelledby={id}
        tabIndex={-1}
      >
        <div className="ph-sheet__handle" aria-hidden="true" />
        {head ?? (
          <div className="ph-sheet__head">
            <div className="ph-sheet__heading">
              <h2 id={id} className="ph-sheet__title">
                {title}
              </h2>
              {subtitle && <p className="ph-sheet__sub">{subtitle}</p>}
            </div>
            {action}
          </div>
        )}
        {head && (
          <h2 id={id} className="sr-only">
            {title}
          </h2>
        )}
        <div className="ph-sheet__body">{children}</div>
        {footer && <div className="ph-sheet__foot">{footer}</div>}
      </div>
    </div>
  );
}

/** A thin progress line: only it follows the position, so its parent stays still on each tick. */
export function ProgressLine({ className }: { className: string }) {
  const progress = usePlayer((s) => (s.durationMs > 0 ? Math.min(1, s.positionMs / s.durationMs) : 0));
  return (
    <span className={className} aria-hidden="true">
      <span className={`${className}-fill`} style={{ transform: `scaleX(${progress})` }} />
    </span>
  );
}

/**
 * One line that may be too long: it sits still and fades at the edge, and
 * while `moving` it drifts slowly sideways to show the rest, resting at each
 * end. Under reduced motion it never moves.
 */
export function Marquee({ text, moving, className = "" }: { text: string; moving: boolean; className?: string }) {
  const box = useRef<HTMLSpanElement>(null);
  const inner = useRef<HTMLSpanElement>(null);
  const [over, setOver] = useState(0);
  useEffect(() => {
    const measure = () => {
      const b = box.current;
      const i = inner.current;
      if (!b || !i) return;
      setOver(Math.max(0, Math.ceil(i.scrollWidth - b.clientWidth)));
    };
    measure();
    if (typeof ResizeObserver === "undefined" || !box.current) return;
    const ro = new ResizeObserver(measure);
    ro.observe(box.current);
    return () => ro.disconnect();
  }, [text]);
  const run = moving && over > 0 && !prefersReducedMotion();
  // About 24 pt a second, plus a rest at each end.
  const seconds = Math.max(6, over / 24 + 4);
  const style = { "--ph-marquee-shift": `-${over}px`, "--ph-marquee-time": `${seconds}s` } as CSSProperties;
  return (
    <span ref={box} className={`ph-marquee ${over > 0 ? "is-over" : ""} ${run ? "is-moving" : ""} ${className}`} style={style}>
      <span ref={inner} className="ph-marquee__text">
        {text}
      </span>
    </span>
  );
}

/** "-1:23": what's left of a song. */
export function remaining(positionMs: number, durationMs: number): string {
  const left = Math.max(0, durationMs - positionMs);
  const s = Math.floor(left / 1000);
  return `-${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}
