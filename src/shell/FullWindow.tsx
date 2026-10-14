import { Maximize, MicVocal, Minimize, X } from "lucide-react";
import { useEffect, useRef, useState, type CSSProperties, type MouseEvent } from "react";
import { Art } from "../components/Art";
import { ArtistLinks, TextLink } from "../components/ArtistLinks";
import { IconButton } from "../components/Button";
import { useLyricsBackground } from "../pages/LyricsPage";
import { fullWindow, FULL_WINDOW_WIDE, useFullWindow } from "../stores/fullWindow";
import { player, usePlayer } from "../stores/player";
import { nav, useRouter } from "../stores/router";
import { useSettings } from "../stores/settings";
import { useUi } from "../stores/ui";
import { timedLines, useLyrics } from "./lyrics";
import { LyricsView } from "./LyricsView";
import { PlayControls, SeekBar, SEEK_KEY_STEP, Volume } from "./PlayerBar";
import { VOLUME_KEY_STEP } from "./useShortcuts";
import "./full-window.css";

/** Controls fade after this long without the mouse moving or a key. */
export const CONTROLS_HIDE_MS = 3_000;
/** Requested size of the big cover. */
const COVER_ART = 640;

function useWindowWidth() {
  const [w, setW] = useState(() => window.innerWidth);
  useEffect(() => {
    const on = () => setW(window.innerWidth);
    window.addEventListener("resize", on);
    return () => window.removeEventListener("resize", on);
  }, []);
  return w;
}

/** True after `ms` without mouse movement, a press or a key (never while `enabled` is off). */
function useIdle(enabled: boolean, ms: number): boolean {
  const [idle, setIdle] = useState(false);
  useEffect(() => {
    if (!enabled) {
      setIdle(false);
      return;
    }
    let t: ReturnType<typeof setTimeout>;
    const wake = () => {
      setIdle(false);
      clearTimeout(t);
      t = setTimeout(() => setIdle(true), ms);
    };
    wake();
    const opts = { capture: true, passive: true } as const;
    window.addEventListener("mousemove", wake, opts);
    window.addEventListener("pointerdown", wake, opts);
    window.addEventListener("keydown", wake, opts);
    window.addEventListener("wheel", wake, opts);
    return () => {
      clearTimeout(t);
      window.removeEventListener("mousemove", wake, opts);
      window.removeEventListener("pointerdown", wake, opts);
      window.removeEventListener("keydown", wake, opts);
      window.removeEventListener("wheel", wake, opts);
    };
  }, [enabled, ms]);
  return idle;
}

const inSlider = (t: EventTarget | null) => t instanceof Element && !!t.closest("[role=slider]");
const inButton = (t: EventTarget | null) => t instanceof Element && !!t.closest("button, a, [role=slider], [role=switch], input, textarea, select");
const modified = (e: KeyboardEvent) => e.ctrlKey || e.metaKey || e.altKey || e.shiftKey;

/**
 * Full-window keys, ahead of the app's own shortcuts: Space play/pause,
 * ←/→ seek 5 s, ↑/↓ volume, L lyrics, F OS full screen, Esc leave (full
 * screen first).
 */
function onFullWindowKey(e: KeyboardEvent) {
  if (e.defaultPrevented || useUi.getState().menu || document.querySelector("dialog[open]")) return;
  const key = e.key;
  let handled = true;
  if (key === "Escape") void fullWindow.escape();
  else if (key === " " && !e.repeat && !inButton(e.target)) player.toggle();
  else if ((key === "ArrowLeft" || key === "ArrowRight") && !modified(e) && !inSlider(e.target))
    void player.seekBy(key === "ArrowRight" ? SEEK_KEY_STEP : -SEEK_KEY_STEP);
  else if ((key === "ArrowUp" || key === "ArrowDown") && !modified(e) && !inSlider(e.target) && !(e.target as Element | null)?.closest?.(".lyrics__scroll"))
    player.nudgeVolume(key === "ArrowUp" ? VOLUME_KEY_STEP : -VOLUME_KEY_STEP);
  else if (key.toLowerCase() === "l" && !modified(e)) fullWindow.toggleLyrics();
  else if (key.toLowerCase() === "f" && !modified(e)) void fullWindow.toggleOsFullscreen();
  else handled = false;
  if (handled) {
    e.preventDefault();
    e.stopPropagation();
  }
}

function FullWindowView() {
  const track = usePlayer((s) => s.state?.track);
  const ctx = usePlayer((s) => s.state?.queue.context);
  const { lyrics } = useLyrics(track?.id);
  const lyricsOn = useFullWindow((s) => s.lyrics);
  const osFull = useFullWindow((s) => s.osFullscreen);
  const hideControls = useSettings((s) => s.settings?.fullWindowHideControls ?? true);
  const idle = useIdle(hideControls, CONTROLS_HIDE_MS);
  const wide = useWindowWidth() >= FULL_WINDOW_WIDE;
  const tint = useLyricsBackground(track?.thumb);
  const root = useRef<HTMLDivElement>(null);

  // Keys go here first; focus moves in and comes back on close.
  useEffect(() => {
    const returnTo = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    root.current?.focus();
    window.addEventListener("keydown", onFullWindowKey, true);
    // Leaving full screen some other way (an OS gesture) keeps the state true.
    const onResize = () => void fullWindow.syncOs();
    window.addEventListener("resize", onResize);
    document.addEventListener("fullscreenchange", onResize);
    return () => {
      window.removeEventListener("keydown", onFullWindowKey, true);
      window.removeEventListener("resize", onResize);
      document.removeEventListener("fullscreenchange", onResize);
      returnTo?.focus?.();
    };
  }, []);

  // A link (album, artist) takes you to its page: the view steps aside.
  useEffect(() => useRouter.subscribe((s, p) => s.index !== p.index && void fullWindow.close()), []);

  const hasLyrics = !!lyrics;
  const showLyrics = hasLyrics && lyricsOn;
  // Narrow windows: lyrics take the cover's place, with a small cover beside the title.
  const compact = showLyrics && !wide;

  const onDoubleClick = (e: MouseEvent) => {
    const t = e.target as Element;
    if (t.closest("button, a, [role=slider], .lyrics, .fw__controls, .fw__top")) return;
    void fullWindow.close();
  };

  return (
    <div
      ref={root}
      className={`fw ${idle ? "is-idle" : ""} ${showLyrics ? "has-lyrics" : ""} ${compact ? "is-compact" : ""} ${wide ? "is-wide" : ""}`}
      role="dialog"
      aria-modal="true"
      aria-label="Now playing, full window"
      tabIndex={-1}
      style={{ "--fw-tint": tint } as CSSProperties}
      onDoubleClick={onDoubleClick}
    >
      <div className="fw__bg" aria-hidden="true">
        {track?.thumb && <Art path={track.thumb} fluid requestSize={COVER_ART} />}
      </div>
      <div className="fw__shade" aria-hidden="true" />

      <header className="fw__top fw__chrome">
        <span className="fw__context truncate">{ctx?.title || track?.albumTitle || "Now playing"}</span>
        <div className="fw__actions">
          <IconButton
            label="Lyrics"
            tooltip={hasLyrics ? "Lyrics (L)" : "No lyrics for this song"}
            tooltipPlacement="bottom"
            pressed={showLyrics}
            softDisabled={!hasLyrics}
            icon={<MicVocal size={18} />}
            onClick={() => fullWindow.toggleLyrics()}
          />
          <IconButton
            label={osFull ? "Exit full screen" : "Enter full screen"}
            tooltip={`${osFull ? "Exit full screen" : "Enter full screen"} (F)`}
            tooltipPlacement="bottom"
            icon={osFull ? <Minimize size={18} /> : <Maximize size={18} />}
            onClick={() => void fullWindow.toggleOsFullscreen()}
          />
          <IconButton
            label="Close full-window view"
            tooltip="Close (Esc)"
            tooltipPlacement="bottom"
            icon={<X size={20} />}
            onClick={() => void fullWindow.close()}
          />
        </div>
      </header>

      <div className="fw__stage">
        {track ? (
          <>
            <div className="fw__now" key={track.id}>
              <div className="fw__cover">
                <Art path={track.thumb} fluid requestSize={COVER_ART} />
              </div>
              <div className="fw__meta">
                <div className="fw__title truncate">
                  <TextLink onActivate={() => nav.go({ name: "album", id: track.albumId })}>{track.title}</TextLink>
                </div>
                <ArtistLinks track={track} className="fw__artists truncate" />
                <div className="fw__album truncate">
                  <TextLink onActivate={() => nav.go({ name: "album", id: track.albumId })}>{track.albumTitle}</TextLink>
                </div>
              </div>
            </div>
            {showLyrics && lyrics && (
              <div className={`fw__lyrics ${timedLines(lyrics) ? "" : "is-plain"}`}>
                <LyricsView lyrics={lyrics} size="full" trackId={track.id} />
              </div>
            )}
          </>
        ) : (
          <p className="fw__empty">Nothing playing.</p>
        )}
      </div>

      <footer className="fw__controls fw__chrome">
        <SeekBar />
        <div className="fw__buttons">
          <div className="fw__spacer" />
          <PlayControls />
          <div className="fw__volume">
            <Volume />
          </div>
        </div>
      </footer>
    </div>
  );
}

/** Large art over a blurred, tinted copy, with title, controls and lyrics; covers the whole window. */
export function FullWindow() {
  const open = useFullWindow((s) => s.open);
  return open ? <FullWindowView /> : null;
}
