/**
 * The miniplayer window's page: big art on a card tinted from the cover,
 * the title, artist and like control under it, and a top strip with
 * settings, a drag handle and close. Pointing at (or tabbing into) the art
 * brings up a dark scrim with one row of controls and the progress along
 * its bottom edge; pointing at mute pops up a vertical volume slider. The
 * settings icon swaps the content for a small settings panel.
 *
 * It runs in its own window with its own copy of the stores, fed by the
 * same core events as the main window.
 */
import {
  GripHorizontal,
  Link,
  Pause,
  Play,
  Repeat,
  Repeat1,
  Shuffle,
  SkipBack,
  SkipForward,
  SlidersHorizontal,
  Sparkles,
  Volume1,
  Volume2,
  VolumeX,
  X,
} from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent, type PointerEvent } from "react";
import { Art } from "../components/Art";
import { Button, IconButton } from "../components/Button";
import { PlusGlyph, TickGlyph } from "../components/SaveButton";
import { Switch } from "../components/Switch";
import { Toaster } from "../components/Toast";
import { backend } from "../lib/backend";
import { NEUTRAL_HEADER } from "../lib/color";
import { formatDuration, trackArtistLine } from "../lib/format";
import type { QueueItem, Track } from "../lib/types";
import { setLiked, useIsLiked } from "../stores/likes";
import { player, usePlayer } from "../stores/player";
import { copyLinksAt, copyLink } from "../stores/share";
import { updateSettings, useSettings } from "../stores/settings";
import { useCoverColors } from "./nowPlaying";
import "./miniplayer.css";

/** The up-next list shows under the art once the window is at least this tall. */
export const QUEUE_MIN_HEIGHT = 480;
/** Songs listed under the art at that height (taller windows list more). */
export const QUEUE_ROWS = 4;

/** The next songs after the current one: the user's queue first, then the context. */
export function upNext(q: { userQueue: QueueItem[]; nextUp: QueueItem[] } | null | undefined, n = QUEUE_ROWS): QueueItem[] {
  if (!q) return [];
  return [...q.userQueue, ...q.nextUp].slice(0, n);
}

/**
 * Track an element's size. Returns a callback ref, so an element that is
 * swapped out (the settings panel replaces the art and puts it back) is
 * measured again instead of the old, detached one reading 0 forever.
 */
function useSize<T extends HTMLElement>(): [(el: T | null) => void, { w: number; h: number }] {
  const [el, setEl] = useState<T | null>(null);
  const [size, setSize] = useState({ w: 0, h: 0 });
  useLayoutEffect(() => {
    if (!el) return;
    const read = () => setSize((s) => (s.w === el.clientWidth && s.h === el.clientHeight ? s : { w: el.clientWidth, h: el.clientHeight }));
    read();
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(read);
    ro.observe(el);
    return () => ro.disconnect();
  }, [el]);
  return [setEl, size];
}

/** Wide windows put the title and queue beside the art from this width up. */
export const WIDE_MIN_WIDTH = 560;
/** ...when the window is at least this much wider than it is tall. */
export const WIDE_RATIO = 1.25;

/**
 * How the window lays out at a size: side by side when wide, else stacked;
 * and how many up-next rows fit (0: none) when the queue is on. A taller
 * window lists more songs instead of only growing the art.
 */
export function miniLayout(w: number, h: number, queueOn: boolean): { wide: boolean; queueRows: number } {
  const wide = w >= WIDE_MIN_WIDTH && w >= h * WIDE_RATIO;
  if (!queueOn) return { wide, queueRows: 0 };
  if (wide) return { wide, queueRows: h >= 300 ? Math.min(10, Math.max(2, Math.floor((h - 200) / 44))) : 0 };
  if (h < QUEUE_MIN_HEIGHT) return { wide, queueRows: 0 };
  // Rows past the first few only use height the full-width art can't.
  const spare = h - w - 120;
  return { wide, queueRows: Math.min(8, Math.max(QUEUE_ROWS, Math.floor(spare / 44))) };
}

/** A vertical volume slider in a small pill (opens over the mute button). */
function VolumePop({ onLeave }: { onLeave: () => void }) {
  const volume = usePlayer((s) => s.state?.volume ?? 0.7);
  const muted = usePlayer((s) => s.state?.muted ?? false);
  const [local, setLocal] = useState<number | null>(null);
  const v = local ?? (muted ? 0 : volume);
  const track = useRef<HTMLDivElement>(null);
  const at = (e: PointerEvent) => {
    const r = track.current!.getBoundingClientRect();
    return Math.min(1, Math.max(0, (r.bottom - e.clientY) / Math.max(1, r.height)));
  };
  const onDown = (e: PointerEvent<HTMLDivElement>) => {
    e.currentTarget.setPointerCapture?.(e.pointerId);
    const x = at(e);
    setLocal(x);
    void player.volumeInput(x);
  };
  const onMove = (e: PointerEvent<HTMLDivElement>) => {
    if (local === null) return;
    const x = at(e);
    setLocal(x);
    void player.volumeInput(x);
  };
  const onUp = (e: PointerEvent<HTMLDivElement>) => {
    if (local === null) return;
    const x = at(e);
    setLocal(null);
    void player.volumeCommit(x);
  };
  const onKey = (e: KeyboardEvent) => {
    const step = e.key === "ArrowUp" || e.key === "ArrowRight" ? 0.1 : e.key === "ArrowDown" || e.key === "ArrowLeft" ? -0.1 : 0;
    if (!step) return;
    e.preventDefault();
    void player.nudgeVolume(step);
  };
  return (
    <div className="mini-volume" onPointerLeave={onLeave}>
      <div
        ref={track}
        className="mini-volume__track"
        role="slider"
        tabIndex={0}
        aria-label="Volume"
        aria-orientation="vertical"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(v * 100)}
        aria-valuetext={`${Math.round(v * 100)}%`}
        onPointerDown={onDown}
        onPointerMove={onMove}
        onPointerUp={onUp}
        onKeyDown={onKey}
      >
        <span className="mini-volume__fill" style={{ height: `${v * 100}%` }} />
        <span className="mini-volume__knob" style={{ bottom: `${v * 100}%` }} />
      </div>
    </div>
  );
}

/** The scrim's controls row and the progress along the art's bottom edge. */
function Controls({ track }: { track: Track }) {
  const state = usePlayer((s) => s.state);
  const positionMs = usePlayer((s) => s.positionMs);
  const durationMs = usePlayer((s) => s.durationMs);
  const [volumeOpen, setVolumeOpen] = useState(false);
  const smartAllowed = useSettings((s) => !!s.settings?.smartShuffle);
  const playing = state?.status === "playing" || state?.status === "buffering" || state?.status === "loading";
  const repeat = state?.repeat ?? "off";
  const shuffle = state?.shuffle ?? false;
  const smart = !!state?.smartShuffle;
  const silent = !!state?.muted || (state?.volume ?? 1) <= 0;
  const VolIcon = silent ? VolumeX : (state?.volume ?? 1) < 0.5 ? Volume1 : Volume2;
  const pct = durationMs > 0 ? Math.min(100, (positionMs / durationMs) * 100) : 0;
  const share = () => {
    const at = Math.floor(positionMs / 1000);
    const art = track.thumb ? { path: track.thumb } : undefined;
    return at > 0 ? copyLinksAt("track", track.id, at, art) : copyLink(backend.shareInfo("track", track.id).then((i) => i.appLink), undefined, art);
  };
  return (
    <>
      <div className="mini__row">
        <span className="mini__mute" onPointerEnter={() => setVolumeOpen(true)}>
          {volumeOpen && <VolumePop onLeave={() => setVolumeOpen(false)} />}
          <IconButton
            label={silent ? "Unmute" : "Mute"}
            tooltip={false}
            icon={<VolIcon size={18} />}
            onClick={() => player.toggleMute()}
            onFocus={() => setVolumeOpen(true)}
          />
        </span>
        <IconButton
          label={smart ? "Disable smart shuffle" : shuffle ? "Disable shuffle" : "Enable shuffle"}
          tooltip={false}
          pressed={shuffle}
          icon={
            smart ? (
              <span className="smart-shuffle-icon">
                <Shuffle size={18} />
                <Sparkles size={10} className="smart-shuffle-icon__spark" />
              </span>
            ) : (
              <Shuffle size={18} />
            )
          }
          onClick={() => player.toggleShuffle(smartAllowed)}
        />
        <IconButton label="Previous" tooltip={false} icon={<SkipBack size={20} fill="currentColor" />} onClick={() => player.previous()} />
        <IconButton
          label={playing ? "Pause" : "Play"}
          tooltip={false}
          size="xl"
          variant="accent"
          className="mini__play"
          icon={playing ? <Pause size={24} fill="currentColor" /> : <Play size={24} fill="currentColor" className="nudge-right" />}
          onClick={() => player.toggle()}
        />
        <IconButton label="Next" tooltip={false} icon={<SkipForward size={20} fill="currentColor" />} onClick={() => player.next()} />
        <IconButton
          label={repeat === "off" ? "Enable repeat" : repeat === "all" ? "Enable repeat one" : "Disable repeat"}
          tooltip={false}
          pressed={repeat !== "off"}
          icon={repeat === "one" ? <Repeat1 size={18} /> : <Repeat size={18} />}
          onClick={() => player.cycleRepeat(repeat)}
        />
        <IconButton label="Copy link" tooltip={false} icon={<Link size={18} />} onClick={() => void share()} />
      </div>
      <div className="mini__progress">
        <div className="mini__times tabular" aria-hidden="true">
          <span>{formatDuration(positionMs)}</span>
          <span>{formatDuration(durationMs)}</span>
        </div>
        <div
          className="mini__bar"
          role="progressbar"
          aria-label="Song progress"
          aria-valuemin={0}
          aria-valuemax={Math.round(durationMs / 1000)}
          aria-valuenow={Math.round(positionMs / 1000)}
          aria-valuetext={`${formatDuration(positionMs)} of ${formatDuration(durationMs)}`}
          onPointerDown={(e) => {
            const r = e.currentTarget.getBoundingClientRect();
            const f = Math.min(1, Math.max(0, (e.clientX - r.left) / Math.max(1, r.width)));
            if (durationMs > 0) void player.seek(Math.round(f * durationMs));
          }}
        >
          <span className="mini__bar-fill" style={{ width: `${pct}%` }} />
        </div>
      </div>
    </>
  );
}

function LikeControl({ track }: { track: Track }) {
  const liked = useIsLiked(track.id);
  return (
    <button
      type="button"
      className={`save-btn save-btn--bar mini__like ${liked ? "is-liked" : ""}`}
      aria-label={liked ? `Remove ${track.title} from Liked Songs` : `Add ${track.title} to Liked Songs`}
      aria-pressed={liked}
      onClick={() => void setLiked([track], !liked, { toast: false })}
    >
      {liked ? <TickGlyph /> : <PlusGlyph />}
    </button>
  );
}

function Meta({ track }: { track: Track }) {
  return (
    <div className="mini__meta">
      <div className="mini__text">
        <p className="mini__title truncate">{track.title}</p>
        <p className="mini__artist truncate">{trackArtistLine(track)}</p>
      </div>
      <LikeControl track={track} />
    </div>
  );
}

function SettingsPanel({ onDone }: { onDone: () => void }) {
  const s = useSettings((x) => x.settings);
  const tint = s?.miniplayerTint ?? true;
  const queue = s?.miniplayerQueue ?? false;
  return (
    <section className="mini-settings" aria-labelledby="mini-settings-title">
      <h1 id="mini-settings-title" className="mini-settings__title">
        Miniplayer settings
      </h1>
      <div className="mini-settings__row">
        <span id="mini-tint">Background colour</span>
        <Switch checked={tint} labelledBy="mini-tint" onChange={(v) => void updateSettings({ miniplayerTint: v })} />
      </div>
      <div className="mini-settings__row">
        <span id="mini-queue">Show the queue</span>
        <Switch checked={queue} labelledBy="mini-queue" onChange={(v) => void updateSettings({ miniplayerQueue: v })} />
      </div>
      <p className="mini-settings__note">The queue shows when the window is big enough.</p>
      <Button variant="primary" className="mini-settings__done" onClick={onDone}>
        Done
      </Button>
    </section>
  );
}

function UpNext({ items }: { items: QueueItem[] }) {
  if (!items.length) return null;
  return (
    <section className="mini-queue" aria-label="Up next">
      <h2 className="mini-queue__title">Up next</h2>
      <ol className="mini-queue__list">
        {items.map((q) => (
          <li key={q.itemId} className="mini-queue__row">
            <Art path={q.track.thumb} size={32} />
            <span className="mini-queue__text">
              <span className="truncate mini-queue__song">{q.track.title}</span>
              <span className="truncate mini-queue__artist">{trackArtistLine(q.track)}</span>
            </span>
          </li>
        ))}
      </ol>
    </section>
  );
}

/** The miniplayer page (the whole window). */
export function Miniplayer() {
  const track = usePlayer((s) => s.state?.track ?? null);
  const queueView = usePlayer((s) => s.state?.queue);
  const settings = useSettings((s) => s.settings);
  const tintOn = (settings?.miniplayerTint ?? true) && settings?.artworkColours !== false;
  const colors = useCoverColors(tintOn ? track?.thumb : null);
  const tint = tintOn ? (colors?.vivid ?? NEUTRAL_HEADER.vivid) : null;
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [active, setActive] = useState(false);
  const [rootRef, root] = useSize<HTMLDivElement>();
  const [artBox, box] = useSize<HTMLDivElement>();
  const { wide, queueRows } = miniLayout(root.w, root.h, settings?.miniplayerQueue ?? false);
  const next = queueRows > 0 ? upNext(queueView, queueRows) : [];
  const artSize = Math.max(0, Math.floor(Math.min(box.w, box.h)));

  useEffect(() => {
    document.title = track ? `${track.title} · ${trackArtistLine(track)}` : "Platter miniplayer";
  }, [track]);

  const close = () => void backend.miniplayerSet(false).catch(() => {});

  return (
    <div ref={rootRef} className={`mini ${tint ? "is-tinted" : ""}`} style={tint ? ({ "--mini-tint": tint } as React.CSSProperties) : undefined}>
      <header className="mini__strip" data-tauri-drag-region>
        <IconButton
          label={settingsOpen ? "Close miniplayer settings" : "Miniplayer settings"}
          tooltip={false}
          size="sm"
          pressed={settingsOpen}
          icon={<SlidersHorizontal size={14} />}
          onClick={() => setSettingsOpen((o) => !o)}
        />
        <span className="mini__handle" data-tauri-drag-region aria-hidden="true">
          <GripHorizontal size={16} data-tauri-drag-region />
        </span>
        <IconButton label="Close miniplayer" tooltip={false} size="sm" icon={<X size={14} />} onClick={close} />
      </header>
      {settingsOpen ? (
        <SettingsPanel onDone={() => setSettingsOpen(false)} />
      ) : (
        <main className={`mini__body ${wide ? "mini__body--wide" : ""}`}>
          <div className="mini__card">
            <div ref={artBox} className="mini__art-area">
              {track ? (
                <div
                  className={`mini__art ${active ? "is-active" : ""}`}
                  style={{ width: artSize, height: artSize }}
                  data-testid="mini-art"
                  onPointerEnter={() => setActive(true)}
                  onPointerLeave={() => setActive(false)}
                  onFocus={() => setActive(true)}
                  onBlur={(e) => !e.currentTarget.contains(e.relatedTarget as Node | null) && setActive(false)}
                >
                  {artSize > 0 && <Art path={track.thumb} size={artSize} />}
                  <div className="mini__scrim">
                    <Controls track={track} />
                  </div>
                </div>
              ) : (
                <p className="mini__empty">Nothing playing</p>
              )}
            </div>
            {wide ? (
              <div className="mini__side">
                {track && <Meta track={track} />}
                <UpNext items={next} />
              </div>
            ) : (
              track && <Meta track={track} />
            )}
          </div>
          {!wide && <UpNext items={next} />}
        </main>
      )}
      <button
        type="button"
        className="mini__grip"
        aria-label="Resize the miniplayer"
        tabIndex={-1}
        onPointerDown={(e) => {
          e.preventDefault();
          void backend.windowResizeFromCorner().catch(() => {});
        }}
      >
        <svg viewBox="0 0 10 10" width="10" height="10" aria-hidden="true">
          <path d="M9 3L3 9M9 6.5L6.5 9" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round" />
        </svg>
      </button>
      <Toaster />
    </div>
  );
}
