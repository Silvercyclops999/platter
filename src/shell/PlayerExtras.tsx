/**
 * Player bar extras: the A-B loop layer over the seek bar, the "Next:" line
 * under the title, and the volume readout.
 */
import { useEffect, useRef, useState, type KeyboardEvent, type PointerEvent, type ReactNode } from "react";
import { ArtistLinks } from "../components/ArtistLinks";
import { Tooltip } from "../components/Tooltip";
import { formatDuration } from "../lib/format";
import type { AbLoop, Track } from "../lib/types";
import { usePlayer } from "../stores/player";
import { useSettings } from "../stores/settings";
import { useUi } from "../stores/ui";
import { LOOP_KEY_BIG_STEP, LOOP_KEY_STEP, READOUT_HOLD_MS, upNextShown, volumeLabel } from "./playerRules";
import "./player-extras.css";

const pct = (ms: number, durationMs: number) => `${Math.min(100, Math.max(0, (ms / Math.max(durationMs, 1)) * 100))}%`;

// ------------------------------------------------------------------ A-B loop

type Point = "aMs" | "bMs";

function LoopHandle({
  point,
  ms,
  durationMs,
  layer,
  onMove,
  onDone,
  onEscape,
}: {
  point: Point;
  ms: number;
  durationMs: number;
  layer: React.RefObject<HTMLDivElement | null>;
  onMove: (point: Point, ms: number) => void;
  onDone: () => void;
  onEscape: () => void;
}) {
  const [dragging, setDragging] = useState(false);
  const fromX = (clientX: number) => {
    const r = layer.current?.getBoundingClientRect();
    if (!r || r.width <= 0) return ms;
    return Math.min(1, Math.max(0, (clientX - r.left) / r.width)) * durationMs;
  };
  const label = point === "aMs" ? "Loop start" : "Loop end";
  return (
    <div
      role="slider"
      tabIndex={0}
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={durationMs}
      aria-valuenow={Math.round(ms)}
      aria-valuetext={formatDuration(ms)}
      className={`ab-handle ab-handle--${point === "aMs" ? "a" : "b"} ${dragging ? "is-dragging" : ""}`}
      style={{ left: pct(ms, durationMs) }}
      onPointerDown={(e: PointerEvent<HTMLDivElement>) => {
        if (e.button !== 0) return;
        e.preventDefault();
        e.stopPropagation();
        e.currentTarget.focus();
        e.currentTarget.setPointerCapture?.(e.pointerId);
        setDragging(true);
      }}
      onPointerMove={(e) => {
        if (dragging) onMove(point, fromX(e.clientX));
      }}
      onPointerUp={(e) => {
        if (!dragging) return;
        setDragging(false);
        onMove(point, fromX(e.clientX));
        onDone();
      }}
      onPointerCancel={() => {
        if (!dragging) return;
        setDragging(false);
        onDone();
      }}
      onKeyDown={(e: KeyboardEvent<HTMLDivElement>) => {
        const step = e.shiftKey ? LOOP_KEY_BIG_STEP : LOOP_KEY_STEP;
        const by = e.key === "ArrowLeft" || e.key === "ArrowDown" ? -step : e.key === "ArrowRight" || e.key === "ArrowUp" ? step : 0;
        if (e.key === "Escape") {
          e.preventDefault();
          e.stopPropagation();
          onEscape();
          return;
        }
        if (!by) return;
        e.preventDefault();
        e.stopPropagation();
        onMove(point, Math.min(durationMs, Math.max(0, ms + by)));
        onDone();
      }}
    />
  );
}

/**
 * Drawn over the seek bar: the tinted span between A and B and their handles,
 * or the lone A while B hasn't been picked yet. Handles drag (or take arrow
 * keys) and commit when let go.
 */
export function LoopLayer({
  loop,
  pendingA,
  durationMs,
  onChange,
  onClear,
}: {
  loop: AbLoop | null;
  pendingA: number | null;
  durationMs: number;
  onChange: (p: number, q: number) => Promise<unknown> | void;
  onClear: () => void;
}) {
  const layer = useRef<HTMLDivElement>(null);
  const [drag, setDrag] = useState<AbLoop | null>(null);
  const dragRef = useRef<AbLoop | null>(null);
  const shown = drag ?? loop;
  const move = (point: Point, ms: number) => {
    if (!shown) return;
    const d = { ...(dragRef.current ?? shown), [point]: ms };
    dragRef.current = d;
    setDrag(d);
  };
  const done = () => {
    const d = dragRef.current;
    dragRef.current = null;
    if (!d || !loop || (d.aMs === loop.aMs && d.bMs === loop.bMs)) {
      setDrag(null);
      return;
    }
    // The core's answer (or a refusal) replaces the dragged values.
    void Promise.resolve(onChange(d.aMs, d.bMs)).finally(() => setDrag(null));
  };
  if (!shown && pendingA == null) return null;
  const lo = shown ? Math.min(shown.aMs, shown.bMs) : 0;
  const hi = shown ? Math.max(shown.aMs, shown.bMs) : 0;
  return (
    <div className="ab-layer" ref={layer}>
      {shown && (
        <>
          <div className="ab-span" style={{ left: pct(lo, durationMs), width: `calc(${pct(hi, durationMs)} - ${pct(lo, durationMs)})` }} />
          {(["aMs", "bMs"] as const).map((p) => (
            <LoopHandle
              key={p}
              point={p}
              ms={shown[p]}
              durationMs={durationMs}
              layer={layer}
              onMove={move}
              onDone={done}
              onEscape={onClear}
            />
          ))}
        </>
      )}
      {!shown && pendingA != null && <div className="ab-handle ab-handle--a ab-handle--pending" style={{ left: pct(pendingA, durationMs) }} aria-hidden="true" />}
    </div>
  );
}

/** "A-B" chip shown while a loop is set; clicking clears it. */
export function LoopChip({ onClear }: { onClear: () => void }) {
  return (
    <Tooltip label="Clear loop">
      <button type="button" className="ab-chip" aria-label="Clear A-B loop" onClick={onClear}>
        A-B
      </button>
    </Tooltip>
  );
}

// ------------------------------------------------------------------ up next

/**
 * The line under the title: the artists, or near the end of the track
 * "Next: title · artist" (cross-faded). Clicking "Next" opens the queue.
 */
export function PlayerSubline({ track }: { track: Track }) {
  const enabled = useSettings((s) => s.settings?.upNextLine ?? true);
  const next = usePlayer((s) => upNextShown(s.state, s.positionMs, s.durationMs, enabled));
  // Keep the last name while the line fades out.
  const [last, setLast] = useState(next);
  useEffect(() => {
    if (next) setLast(next);
  }, [next]);
  const item = next ?? last;
  const on = !!next;
  const artist = item ? (item.track.trackArtist || item.track.artistTitle) : "";
  return (
    <div className="player-sub">
      <div className={`player-sub__line ${on ? "is-out" : ""}`} aria-hidden={on || undefined}>
        <ArtistLinks track={track} className="player-bar__artist truncate" />
      </div>
      {item && (
        <button
          type="button"
          className={`player-sub__line up-next truncate ${on ? "" : "is-out"}`}
          tabIndex={on ? 0 : -1}
          aria-hidden={!on || undefined}
          onClick={() => useUi.getState().setQueueOpen(true)}
        >
          Next: {item.track.title}
          {artist ? ` · ${artist}` : ""}
        </button>
      )}
    </div>
  );
}

// ------------------------------------------------------------------ volume readout

/**
 * A percentage (or "Muted") above the volume thumb while the level changes,
 * fading out READOUT_HOLD_MS after the last change. Held while dragging.
 */
export function VolumeReadout({ value, silent, dragging, children }: { value: number; silent: boolean; dragging: boolean; children: ReactNode }) {
  const enabled = useSettings((s) => s.settings?.volumePercent ?? true);
  const hasState = usePlayer((s) => !!s.state);
  const [shown, setShown] = useState(false);
  const prev = useRef<string | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const text = volumeLabel(value, silent);
  useEffect(() => {
    if (!hasState) return;
    // The first value is the restored level, not a change.
    if (prev.current === null) {
      prev.current = text;
      return;
    }
    if (prev.current === text) return;
    prev.current = text;
    setShown(true);
    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(() => setShown(false), READOUT_HOLD_MS);
  }, [text, hasState]);
  useEffect(() => () => {
    if (timer.current) clearTimeout(timer.current);
  }, []);
  const visible = enabled && (shown || dragging);
  return (
    <div className="volume__bar">
      {children}
      {enabled && (
        <div className={`volume-readout tabular ${visible ? "is-shown" : ""}`} // Above the thumb, kept from spilling past the window edge at the ends.
          style={{ left: `clamp(22px, ${Math.min(1, Math.max(0, silent ? 0 : value)) * 100}%, calc(100% - 22px))` }} aria-hidden="true">
          {text}
        </div>
      )}
    </div>
  );
}
