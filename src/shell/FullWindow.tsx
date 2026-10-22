import { AudioLines, Disc3, Ellipsis, Maximize, MicVocal, Minimize, Minimize2, RefreshCcw, UserRound } from "lucide-react";
import { useEffect, useRef, useState, type CSSProperties, type MouseEvent, type ReactNode, type RefObject } from "react";
import { Art } from "../components/Art";
import { ArtistLinks, TextLink } from "../components/ArtistLinks";
import { IconButton } from "../components/Button";
import { openContextMenu } from "../components/ContextMenu";
import { trackMenuItems } from "../components/TrackRow";
import type { Track } from "../lib/types";
import { fullWindow, useFullWindow, type FullWindowMode } from "../stores/fullWindow";
import { player, usePlayer } from "../stores/player";
import { nav } from "../stores/router";
import { useSettings } from "../stores/settings";
import { useUi } from "../stores/ui";
import { actionableNotice, useLyrics } from "./lyrics";
import { LyricsPane, useLyricsBackground } from "./LyricsView";
import { contextRoute, useNowPlayingDetails } from "./nowPlaying";
import { SEEK_KEY_STEP } from "./PlayerBar";
import { Turntable } from "./Turntable";
import { useDeckLyrics } from "./deckLyrics";
import { useFlipPause } from "./useFlipPause";
import { openSignal } from "../stores/signalPath";
import { VOLUME_KEY_STEP } from "./useShortcuts";
import { Stage, VisName } from "../visualizer/Stage";
import { stepVisStyle, useCoverTap, useVisStyle } from "../visualizer/useVis";
import type { VisualizerStyle } from "../lib/types";
import "./full-window.css";

/**
 * The header, the player bar and the top bar's contents fade after this long
 * without the mouse moving or a key.
 */
export const CONTROLS_HIDE_MS = 4_000;
/** Requested size of the big cover. */
const COVER_ART = 640;
/** Requested size of the artist image. */
const ARTIST_ART = 1280;

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
/** Typing in a field: letter keys are the field's. */
const editable = (t: EventTarget | null) =>
  t instanceof Element && (!!t.closest("input, textarea, select, [contenteditable=''], [contenteditable='true']") || (t as HTMLElement).isContentEditable === true);

/**
 * Keys while the view is open, ahead of the app's own shortcuts: Space
 * play/pause, ←/→ seek 5 s, ↑/↓ volume, L lyrics (like the lyrics button),
 * F OS full screen, Esc leave (full screen first).
 */
/**
 * Set while the controls are hidden over the art: any key brings them back
 * (and Escape does only that). Returns true when it used up the key.
 */
let revealOnKey: ((e: KeyboardEvent) => boolean) | null = null;

function onFullWindowKey(e: KeyboardEvent) {
  if (e.defaultPrevented || useUi.getState().menu || document.querySelector("dialog[open], .dialog-backdrop")) return;
  if (revealOnKey?.(e)) {
    e.preventDefault();
    e.stopPropagation();
    return;
  }
  // The signal path panel over the view handles its own keys (Esc closes it first).
  if (e.target instanceof Element && e.target.closest(".signal-panel")) return;
  const key = e.key;
  // In a panel beside the view, keys are the panel's own (Esc still leaves the view).
  if (key !== "Escape" && e.target instanceof Element && e.target.closest(".app__body")) return;
  let handled = true;
  const vis = useFullWindow.getState().mode === "visualizer";
  // Over the visualizer, Escape closes it first and the side arrows change its style.
  if (key === "Escape" && vis && !useFullWindow.getState().osFullscreen) fullWindow.setMode("artwork");
  else if (key === "Escape") void fullWindow.escape();
  else if ((key === "ArrowLeft" || key === "ArrowRight") && vis && !modified(e) && !inSlider(e.target)) stepVisStyle(key === "ArrowRight" ? 1 : -1);
  else if (key === " " && !e.repeat && !inButton(e.target)) player.toggle();
  else if ((key === "ArrowLeft" || key === "ArrowRight") && !modified(e) && !inSlider(e.target))
    void player.seekBy(key === "ArrowRight" ? SEEK_KEY_STEP : -SEEK_KEY_STEP);
  else if ((key === "ArrowUp" || key === "ArrowDown") && !modified(e) && !inSlider(e.target) && !(e.target as Element | null)?.closest?.(".lyrics__scroll"))
    player.nudgeVolume(key === "ArrowUp" ? VOLUME_KEY_STEP : -VOLUME_KEY_STEP);
  else if (key.toLowerCase() === "l" && !modified(e) && !editable(e.target)) fullWindow.toggleLyrics();
  else if (key.toLowerCase() === "v" && !modified(e) && !editable(e.target)) fullWindow.toggleVisualizer();
  else if (key.toLowerCase() === "f" && !modified(e) && !editable(e.target)) void fullWindow.toggleOsFullscreen();
  else handled = false;
  if (handled) {
    e.preventDefault();
    e.stopPropagation();
  }
}

/** Title, artists and album; links go to their pages. */
function Meta({ track, className = "" }: { track: Track; className?: string }) {
  return (
    <div className={`fw__meta ${className}`}>
      <div className="fw__title truncate">
        <TextLink onActivate={() => nav.go({ name: "album", id: track.albumId })}>{track.title}</TextLink>
      </div>
      <ArtistLinks track={track} className="fw__artists truncate" />
      <div className="fw__album truncate">
        <TextLink onActivate={() => nav.go({ name: "album", id: track.albumId })}>{track.albumTitle}</TextLink>
      </div>
    </div>
  );
}

function ModeToggle({ mode, current, label, off, offTip, icon }: { mode: FullWindowMode; current: FullWindowMode; label: string; off?: boolean; offTip?: string; icon: ReactNode }) {
  const on = mode === current;
  return (
    <IconButton
      label={label}
      tooltip={off && !on ? offTip : mode === "lyrics" ? `${label} (L)` : mode === "visualizer" ? `${label} (V)` : label}
      tooltipPlacement="bottom"
      pressed={on}
      softDisabled={off && !on}
      icon={icon}
      onClick={() => fullWindow.setMode(mode)}
    />
  );
}

/** What's playing from (album, playlist, artist, Liked Songs), linking back to it when it's a page. */
function ContextName() {
  const ctx = usePlayer((s) => s.state?.queue.context);
  const track = usePlayer((s) => s.state?.track);
  const title = ctx?.title || (ctx?.kind === "album" ? track?.albumTitle : null) || (ctx?.kind === "liked" ? "Liked Songs" : null) || "Now playing";
  const route = contextRoute(ctx);
  return (
    <span className="fw__context truncate">
      {route ? <TextLink onActivate={() => nav.go(route)}>{title}</TextLink> : title}
    </span>
  );
}

/** Root classes the rest of the window reads: the view is open, and its controls are hidden. */
function useRootClasses(idle: boolean) {
  useEffect(() => {
    const root = document.documentElement.classList;
    root.add("fw-open");
    return () => root.remove("fw-open", "fw-idle");
  }, []);
  useEffect(() => {
    document.documentElement.classList.toggle("fw-idle", idle);
  }, [idle]);
}

function FullWindowView() {
  const track = usePlayer((s) => s.state?.track);
  const { lyrics, loading: lyricsLoading, info: lyricsInfo } = useLyrics(track?.id);
  const { details } = useNowPlayingDetails(track?.id);
  const mode = useFullWindow((s) => s.mode);
  const turntable = useSettings((s) => (s.settings?.recordMotion ?? "off") !== "off");
  const playing = usePlayer((s) => s.state?.status === "playing");
  const osFull = useFullWindow((s) => s.osFullscreen);
  const hideControls = useSettings((s) => s.settings?.fullWindowHideControls ?? true);
  const coverTap = useCoverTap();
  const idle = useIdle(hideControls, CONTROLS_HIDE_MS);
  // Over the art, a click hides the controls and another brings them back.
  const [bare, setBare] = useState(false);
  const visStyle = useVisStyle();
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
      // Focus in a panel beside the view stays there.
      if (!document.activeElement || document.activeElement === document.body) returnTo?.focus?.();
    };
  }, []);

  const hasLyrics = !!lyrics || actionableNotice(lyricsInfo);
  const noLyrics = !!track && !lyricsLoading && !hasLyrics;
  // The signal path opens from the player bar's badge, and from here.
  const signalOn = useSettings((s) => s.settings?.signalPathBadge !== false);
  const remote = usePlayer((s) => !!s.remote);
  const signalItem = signalOn && !remote ? [{ label: "Signal path", icon: AudioLines, separator: true, onSelect: () => openSignal() }] : [];
  const artistPhoto = details?.artist ? details.artist.art || details.artist.thumb : null;
  // A song whose artist has no image shows its artwork; the mode stays for the next song.
  const shown: FullWindowMode = mode === "artist" && !artistPhoto ? "artwork" : mode;
  const art: VisualizerStyle | null = shown === "visualizer" && track ? visStyle : null;
  if (bare && !art) setBare(false);
  useRootClasses(idle || bare);
  // Hidden controls are out of reach of Tab too; a key brings them back, and focus never rests on them.
  useEffect(() => {
    document.documentElement.classList.toggle("fw-bare", bare);
    if (!bare) return;
    const el = document.activeElement;
    if (el instanceof HTMLElement && (el.closest(".fw__chrome") || el.closest(".topbar, .player-bar"))) root.current?.focus({ preventScroll: true });
    revealOnKey = (e) => {
      setBare(false);
      return e.key === "Escape";
    };
    return () => {
      revealOnKey = null;
      document.documentElement.classList.remove("fw-bare");
    };
  }, [bare]);
  const artCover = useRef<HTMLDivElement>(null);
  const { flip, play: flipOver } = useFlipPause(turntable && shown === "artwork");

  const onClick = (e: MouseEvent) => {
    const t = e.target as Element;
    if (t.closest("button, a, input, select, textarea, [role=slider], [role=switch], .fw__top")) return;
    // A click on the cover opens the visualizer, and closes it again.
    if (coverTap && shown !== "lyrics" && t.closest(".fw__cover.is-tappable")) {
      if (art) fullWindow.setMode("artwork");
      else fullWindow.open("visualizer");
      return;
    }
    if (art) setBare((b) => !b);
  };

  const onDoubleClick = (e: MouseEvent) => {
    const t = e.target as Element;
    if (t.closest("button, a, [role=slider], .lyrics, .fw__top, .lyrics-page__empty-wrap")) return;
    void fullWindow.close();
  };

  let stage;
  if (!track) stage = <p className="fw__empty">Nothing playing.</p>;
  else if (shown === "lyrics")
    stage = (
      <div className="fw__lyrics-mode" key={track.id}>
        <div className="fw__now fw__now--row fw__chrome">
          <div className="fw__cover">
            <Art path={track.thumb} fluid requestSize={COVER_ART} />
          </div>
          <Meta track={track} />
        </div>
        <div className="fw__lyrics">
          <LyricsPane />
        </div>
      </div>
    );
  else if (art)
    stage = (
      <>
        <div className="fw__now fw__now--art" key="art">
          <div className="fw__art-slot">
            <div ref={artCover} className={`fw__cover vis-cover vis-cover--${art} ${coverTap ? "is-tappable" : ""}`} title={coverTap ? "Close the visualizer" : undefined}>
              <Art path={track.thumb} fluid requestSize={COVER_ART} sleeve={{ title: track.albumTitle, artist: track.artistTitle }} />
            </div>
            <VisName style={art} />
          </div>
          <div className="fw__chrome fw__art-meta" inert={bare || undefined}>
            <Meta track={track} />
          </div>
        </div>
      </>
    );
  else if (shown === "artist" && artistPhoto)
    stage = (
      <div className="fw__artist" key={`${track.id}-artist`}>
        <div className="fw__artist-photo">
          <Art path={artistPhoto} fluid requestSize={ARTIST_ART} kind="artist" />
        </div>
        <div className="fw__artist-meta">
          {details?.artist && (
            <TextLink className="fw__artist-name truncate" onActivate={() => nav.go({ name: "artist", id: details.artist!.id })}>
              {details.artist.title}
            </TextLink>
          )}
          <Meta track={track} />
        </div>
      </div>
    );
  else
    stage = (
      <div className={`fw__now ${turntable ? "fw__now--turntable" : ""}`} key={track.id}>
        {turntable ? (
          <Turntable thumb={track.thumb} sleeve={{ title: track.albumTitle, artist: track.artistTitle }} track={track} playing={playing && !flip} requestSize={COVER_ART}>
            {flip && (
              <>
                <p className="turntable__ended" role="status">
                  End of side {flip.from}
                </p>
                <button type="button" className="turntable__flip" onClick={flipOver}>
                  <RefreshCcw size={16} aria-hidden="true" />
                  Flip to side {flip.to}
                </button>
              </>
            )}
          </Turntable>
        ) : (
          <div className={`fw__cover ${coverTap ? "is-tappable" : ""}`} title={coverTap ? "Open the visualizer (V)" : undefined}>
            <Art path={track.thumb} fluid requestSize={COVER_ART} />
          </div>
        )}
        <Meta track={track} />
      </div>
    );

  return (
    <div
      ref={root}
      className={`fw fw--${shown} ${art ? `fw--art fw--art-${art}` : ""} ${idle ? "is-idle" : ""} ${bare ? "is-bare" : ""}`}
      role="dialog"
      aria-label="Expanded now playing"
      tabIndex={-1}
      style={{ "--fw-tint": tint } as CSSProperties}
      onDoubleClick={onDoubleClick}
      onClick={onClick}
    >
      {art && <FullWindowArt art={art} cover={artCover} />}
      <div className="fw__bg" aria-hidden="true">
        {track?.thumb && <Art path={track.thumb} fluid requestSize={COVER_ART} />}
      </div>
      <div className="fw__shade" aria-hidden="true" />

      <header className="fw__top fw__chrome" inert={bare || undefined}>
        <ContextName />
        <div className="fw__actions">
          <div className="fw__modes" role="group" aria-label="Show">
            <ModeToggle mode="artwork" current={mode} label="Artwork" icon={<Disc3 size={18} />} />
            <ModeToggle mode="artist" current={mode} label="Artist image" off={!artistPhoto} offTip="No artist image" icon={<UserRound size={18} />} />
            <ModeToggle mode="lyrics" current={mode} label="Lyrics" off={noLyrics || !track} offTip="No lyrics for this song" icon={<MicVocal size={18} />} />
            <ModeToggle mode="visualizer" current={shown} label="Visualizer" icon={<AudioLines size={18} />} />
          </div>
          <span className="fw__divider" aria-hidden="true" />
          {track && (
            <IconButton
              label="More options"
              tooltip={`More options for ${track.title}`}
              tooltipPlacement="bottom"
              icon={<Ellipsis size={20} />}
              onClick={(e) => openContextMenu(`Options for ${track.title}`, [...trackMenuItems(track), ...signalItem], e.currentTarget)}
            />
          )}
          <IconButton
            label={osFull ? "Exit full screen" : "Enter full screen"}
            tooltip={`${osFull ? "Exit full screen" : "Enter full screen"} (F)`}
            tooltipPlacement="bottom"
            icon={osFull ? <Minimize size={18} /> : <Maximize size={18} />}
            onClick={() => void fullWindow.toggleOsFullscreen()}
          />
          <IconButton
            label="Minimize expanded view"
            tooltip="Minimize (Esc)"
            tooltipPlacement="bottom"
            icon={<Minimize2 size={18} />}
            onClick={() => void fullWindow.close()}
          />
        </div>
      </header>

      <div className="fw__stage">{stage}</div>
    </div>
  );
}

/** The visualizer behind the expanded view, round its cover. */
function FullWindowArt({ art, cover }: { art: VisualizerStyle; cover: RefObject<HTMLDivElement | null> }) {
  const [find] = useState(() => () => cover.current);
  return <Stage style={art} cover={find} />;
}

/**
 * The expanded now-playing view: fills the window between the top bar and
 * the player bar (the sidebar and the now-playing panel go under it) with
 * the artwork, the artist's image or the lyrics, picked in its header. The
 * queue, the device picker and friend activity open beside it, and it
 * narrows to share the window. Minimize or Esc returns to the layout
 * underneath, the panel beside it still open. Going to a page leaves it too.
 */
export function FullWindow() {
  const open = useFullWindow((s) => s.open);
  // Deck shows lyrics as a page under its faceplate instead.
  const asPage = useDeckLyrics();
  return open && !asPage ? <FullWindowView /> : null;
}
