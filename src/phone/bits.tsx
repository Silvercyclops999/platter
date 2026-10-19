/**
 * Small pieces the phone screens share: the format plate, the bottom sheet
 * frame, row glyphs, a like control and a light tap of feedback.
 */
import { Heart, Pin } from "lucide-react";
import { createContext, useContext, useEffect, useId, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { Plate } from "../components/Plate";
import { backend, type HapticKind } from "../lib/backend";
import { prefersReducedMotion } from "../design/motion";
import { usePlayer } from "../stores/player";
import { ExplicitBadge } from "../components/FileTagBadges";
import type { DownloadKind } from "../lib/types";
import { useAlbumBadges, useTrackBadges } from "../stores/fileTags";
import { DownloadMark, ItemDownloadMark, TrackDownloadMark } from "./DownloadMark";

import { qualityLabel } from "../lib/format";
import { isLosslessCodec } from "../lib/libraryChips";
import type { Track } from "../lib/types";

import { setLiked, useIsLiked } from "../stores/likes";
import { useSettings } from "../stores/settings";
import { phone } from "./phoneUi";

/** Taps closer together than this buzz once. */
export const HAPTIC_GAP_MS = 120;
let lastHaptic = -Infinity;

/** Forget the last buzz (tests). */
export function resetHaptics() {
  lastHaptic = -Infinity;
}

/**
 * A tap of feedback from the phone's haptic engine, while Haptics on
 * controls is on: light for play, pause, skip and like, selection for tabs,
 * pills and sheet choices, success when a song lands in a playlist. A
 * second tap within HAPTIC_GAP_MS doesn't buzz again, unless `always` (a
 * result that must be felt even right after a tick). Off the phone, or on a
 * core without haptics, nothing happens.
 */
export function tap(kind: HapticKind = "light", always = false) {
  if (useSettings.getState().settings?.phoneHaptics === false) return;
  const now = typeof performance !== "undefined" ? performance.now() : Date.now();
  if (!always && now - lastHaptic < HAPTIC_GAP_MS) return;
  lastHaptic = now;
  try {
    void backend.haptic(kind).catch(() => undefined);
  } catch {
    // No haptics here.
  }
}

/**
 * What the plate says: the format of a lossless file ("FLAC 24/96"). A
 * lossy file's bitrate is left off the phone: it is the usual case, and a
 * number there reads like a debug label. Downloaded is the row glyph, not
 * a word on the plate.
 */
export function plateText(track: Pick<Track, "codec" | "bitDepth" | "sampleRate" | "bitrate">): string | null {
  return isLosslessCodec(track.codec) ? qualityLabel(track) : null;
}

/** An album's plate: its format when every song shares one lossless format. */
export function albumPlate(tracks: Pick<Track, "codec" | "bitDepth" | "sampleRate" | "bitrate">[]): string | null {
  const labels = new Set(tracks.map((t) => plateText(t)));
  if (labels.size !== 1) return null;
  return [...labels][0];
}

/** The engraved format plate (lossless only). */
export function FormatPlate({ track, className = "" }: { track: Track; className?: string }) {
  const text = plateText(track);
  if (!text) return null;
  const hiRes = (track.bitDepth ?? 0) > 16 || (track.sampleRate ?? 0) > 48_000;
  return (
    <Plate size="sm" tone={hiRes ? "lit" : "plain"} className={`ph-plate ${className}`}>
      {text}
    </Plate>
  );
}

/**
 * The small glyphs before a row's grey line, in one order and one size:
 * pinned, explicit, then the download mark. A song row passes `trackId`
 * (its explicit tag and its own download); an album row `albumId` (the
 * album's explicit tag); a library row its download `item`. Nothing shows
 * when none applies.
 */
export function RowGlyphs({
  pinned,
  downloaded,
  trackId,
  albumId,
  item,
}: {
  pinned?: boolean;
  downloaded?: boolean;
  trackId?: string;
  albumId?: string;
  item?: { kind: DownloadKind; id: string };
}) {
  return (
    <span className="ph-glyphs">
      {pinned && (
        <span className="ph-glyph ph-glyph--pin" role="img" aria-label="Pinned">
          <Pin size={12} aria-hidden="true" />
        </span>
      )}
      {trackId && <TrackExplicit trackId={trackId} />}
      {albumId && <AlbumExplicit albumId={albumId} />}
      {trackId ? (
        <TrackDownloadMark trackId={trackId} />
      ) : item ? (
        <ItemDownloadMark kind={item.kind} id={item.id} downloaded={downloaded} />
      ) : (
        downloaded && <DownloadMark state="done" />
      )}
    </span>
  );
}

/** The desktop's "E", at the row glyphs' size. */
function TrackExplicit({ trackId }: { trackId: string }) {
  return useTrackBadges(trackId).explicit ? <ExplicitBadge /> : null;
}

function AlbumExplicit({ albumId }: { albumId: string }) {
  return useAlbumBadges(albumId)?.explicit ? <ExplicitBadge /> : null;
}

/**
 * Like a song: the heart fills with a short pop and a light tap, and that
 * is the whole answer (no toast). Once liked, the filled heart opens the
 * song's sheet to unlike it or add it to a playlist.
 */
export function LikeControl({ track, size = 24 }: { track: Track; size?: number }) {
  const liked = useIsLiked(track.id);
  const [pop, setPop] = useState(0);
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
        else {
          setPop((n) => n + 1);
          void setLiked([track], true, { toast: false });
        }
      }}
    >
      <Heart key={pop} size={size} aria-hidden="true" className={pop && liked ? "ph-like__pop" : undefined} fill={liked ? "currentColor" : "none"} strokeWidth={liked ? 0 : 1.75} />
    </button>
  );
}

/** True for a sheet that has been closed and is sliding away (SheetHost keeps it a moment). */
export const SheetLeaving = createContext(false);

/** How far down a sheet is dragged by its top before letting go closes it. */
export const SHEET_CLOSE_PX = 90;

/**
 * A bottom sheet: a handle, a heading, and its body over the dimmed screen.
 * Escape goes back to the sheet under it; tapping the dim area closes every
 * sheet (or only this one with `dimBack`, for a sheet stacked on another).
 * Dragging the top of the sheet down closes it. The frame draws on the tap;
 * its body follows a frame later so the slide starts at once.
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
  dimBack = false,
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
  /** Tapping the dim area goes back to the sheet under this one. */
  dimBack?: boolean;
}) {
  const id = useId();
  const leaving = useContext(SheetLeaving);
  const box = useRef<HTMLDivElement>(null);
  const [ready, setReady] = useState(typeof requestAnimationFrame === "undefined" || import.meta.env?.MODE === "test");
  useEffect(() => {
    if (ready) return;
    const f = requestAnimationFrame(() => setReady(true));
    return () => cancelAnimationFrame(f);
  }, [ready]);
  // Dragging the top of the sheet down.
  const drag = useRef<{ y: number; id: number } | null>(null);
  const [dy, setDy] = useState(0);
  const grab = {
    onPointerDown: (e: React.PointerEvent<HTMLElement>) => {
      if ((e.target as HTMLElement).closest("button, a, input")) return;
      drag.current = { y: e.clientY, id: e.pointerId };
      e.currentTarget.setPointerCapture?.(e.pointerId);
    },
    onPointerMove: (e: React.PointerEvent<HTMLElement>) => {
      if (drag.current?.id !== e.pointerId) return;
      setDy(Math.max(0, e.clientY - drag.current.y));
    },
    onPointerUp: (e: React.PointerEvent<HTMLElement>) => {
      if (drag.current?.id !== e.pointerId) return;
      const far = e.clientY - drag.current.y;
      drag.current = null;
      setDy(0);
      if (far > SHEET_CLOSE_PX) onClose();
    },
    onPointerCancel: () => {
      drag.current = null;
      setDy(0);
    },
  };
  useEffect(() => {
    // preventScroll: focusing the sheet while it is still below the screen
    // would make the browser scroll the whole phone up to reveal it.
    box.current?.focus({ preventScroll: true });
  }, []);
  useEffect(() => {
    if (leaving) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, leaving]);
  return (
    <div className={`ph-sheet-layer ${leaving ? "is-closing" : ""}`} aria-hidden={leaving || undefined} inert={leaving || undefined}>
      <div className="ph-sheet-dim" data-testid="sheet-dim" onClick={dimBack ? onClose : phone.closeSheets} aria-hidden="true" />
      <div
        ref={box}
        className={`ph-sheet ${tall ? "ph-sheet--tall" : ""} ${dy ? "is-dragging" : ""} ${className}`}
        role="dialog"
        aria-modal="true"
        aria-labelledby={id}
        tabIndex={-1}
        style={dy && !leaving ? { transform: `translateY(${dy}px)` } : undefined}
      >
        <div className="ph-sheet__grab" {...grab}>
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
        </div>
        {head && (
          <h2 id={id} className="sr-only">
            {title}
          </h2>
        )}
        <div className="ph-sheet__body">{ready ? children : null}</div>
        {footer && <div className="ph-sheet__foot">{ready ? footer : null}</div>}
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
