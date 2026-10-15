import {
  EyeOff,
  ListMusic,
  Maximize2,
  MicVocal,
  MonitorSpeaker,
  PictureInPicture2,
  Pause,
  Play,
  Repeat,
  Repeat1,
  Shuffle,
  Sparkles,
  SkipBack,
  SkipForward,
  Volume1,
  Volume2,
  VolumeX,
} from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Art } from "../components/Art";
import { TextLink } from "../components/ArtistLinks";
import { ExplicitMark } from "../components/FileTagBadges";
import { openContextMenu } from "../components/ContextMenu";
import { SaveButton } from "../components/SaveButton";
import { trackMenuItems } from "../components/TrackRow";
import { IconButton } from "../components/Button";
import { Skeleton } from "../components/Skeleton";
import { Slider } from "../components/Slider";
import { Tooltip } from "../components/Tooltip";
import { formatDuration, trackArtistLine } from "../lib/format";
import { modKey } from "../lib/platform";
import { dropOnQueue, endSongDrag, startSongDrag, tracksDrag, useSongDrop } from "../stores/drag";
import { player, usePlayer } from "../stores/player";
import { fullWindow, useFullWindow } from "../stores/fullWindow";
import { miniplayer, useMiniplayer } from "../stores/miniplayer";
import { nav } from "../stores/router";
import { useSettings } from "../stores/settings";
import { useUi } from "../stores/ui";
import { LoopChip, LoopLayer, PlayerSubline, VolumeReadout } from "./PlayerExtras";
import { useLyrics } from "./lyrics";
import { normalizeLoop } from "./playerRules";

/** Seek bar keys: arrows move 5 s, Page Up/Down 25 s. */
export const SEEK_KEY_STEP = 5_000;
export const SEEK_PAGE_STEP = 25_000;

export function SeekBar() {
  const positionMs = usePlayer((s) => s.positionMs);
  const durationMs = usePlayer((s) => s.durationMs);
  const trackId = usePlayer((s) => s.state?.track?.id ?? null);
  const hasTrack = trackId !== null;
  const loopOn = useSettings((s) => s.settings?.abLoopHandles ?? true);
  const loop = usePlayer((s) => s.state?.abLoop ?? null);
  const [drag, setDrag] = useState<number | null>(null);
  // A-B loop: the first right-click (or Alt+click) picks A, the second B.
  const [pendingA, setPendingA] = useState<number | null>(null);
  useEffect(() => setPendingA(null), [trackId, loopOn]);
  const shown = drag ?? positionMs;
  const clearLoop = () => {
    setPendingA(null);
    if (loop) void player.setAbLoop(null);
  };
  const setLoop = (p: number, q: number) => {
    const next = normalizeLoop(p, q, durationMs);
    return next ? player.setAbLoop(next) : undefined;
  };
  const onAltPress = (v: number) => {
    const ms = Math.round(v);
    if (pendingA == null) {
      // Starting over: a set loop goes and this point is the new A.
      if (loop) void player.setAbLoop(null);
      setPendingA(ms);
      return;
    }
    // Too close to A: wait for another point.
    if (!normalizeLoop(pendingA, ms, durationMs)) return;
    setPendingA(null);
    void setLoop(pendingA, ms);
  };
  const showLoop = loopOn && hasTrack;
  return (
    <div className="seek">
      {showLoop && loop && <LoopChip onClear={clearLoop} />}
      <span className="seek__time tabular" aria-hidden="true">
        {formatDuration(shown)}
      </span>
      <div className="seek__bar">
        <Slider
          label={hasTrack ? "Seek" : "Seek, nothing playing"}
          value={Math.min(shown, durationMs)}
          max={Math.max(durationMs, 1)}
          step={SEEK_KEY_STEP}
          pageStep={SEEK_PAGE_STEP}
          disabled={!hasTrack}
          valueText={(v) => (hasTrack ? `${formatDuration(v)} of ${formatDuration(durationMs)}` : "Nothing playing")}
          // Pressing seeks at once; while held the time stays at the pointer,
          // and letting go commits where it ended. A press outside a loop's
          // span clears the loop (the core does that on the seek).
          commitOnPress
          onChange={(v) => setDrag(v)}
          onCommit={(v, cause) => {
            if (cause !== "press") setDrag(null);
            player.seek(Math.round(v));
          }}
          onAltPress={showLoop ? onAltPress : undefined}
          onOtherKey={(e) => {
            if (e.key === "Escape" && (loop || pendingA != null)) {
              e.preventDefault();
              e.stopPropagation();
              clearLoop();
            }
          }}
        />
        {showLoop && (
          <LoopLayer loop={loop} pendingA={pendingA} durationMs={durationMs} onChange={setLoop} onClear={clearLoop} />
        )}
      </div>
      <span className="seek__time seek__time--end tabular" aria-hidden="true">
        -{formatDuration(Math.max(0, durationMs - shown))}
      </span>
    </div>
  );
}

/** Volume per mouse wheel notch, and per slider key: 10 points. */
export const WHEEL_STEP = 0.1;
export const VOLUME_SLIDER_STEP = 0.1;
/** Pixels of wheel delta that make one notch (a mouse notch is about 100). */
const WHEEL_NOTCH_PX = 100;

/**
 * Volume change for one wheel event: a mouse notch moves WHEEL_STEP, smooth
 * (trackpad) deltas move in proportion, capped at WHEEL_STEP per event.
 * Wheel up is louder.
 */
export function wheelVolumeDelta(e: { deltaX: number; deltaY: number; deltaMode: number }): number {
  const d = e.deltaY || -e.deltaX;
  if (!d) return 0;
  // Line or page mode (some mice / platforms): treat each event as a notch.
  const size = e.deltaMode === 0 ? Math.min(1, Math.abs(d) / WHEEL_NOTCH_PX) * WHEEL_STEP : WHEEL_STEP;
  return d < 0 ? size : -size;
}

export function Volume() {
  const volume = usePlayer((s) => s.state?.volume ?? 0.7);
  const muted = usePlayer((s) => s.state?.muted ?? false);
  const [local, setLocal] = useState<number | null>(null);
  const v = local ?? (muted ? 0 : volume);
  // A level of 0 counts as muted, so the button always offers to unmute it.
  const silent = muted || v === 0;
  const Icon = silent ? VolumeX : v < 0.5 ? Volume1 : Volume2;
  const box = useRef<HTMLDivElement>(null);
  // Mouse wheel over the volume control nudges it (native listener so the
  // page doesn't scroll instead).
  useEffect(() => {
    const el = box.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      const delta = wheelVolumeDelta(e);
      if (!delta) return;
      e.preventDefault();
      player.nudgeVolume(delta);
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, []);
  return (
    <div className="volume" ref={box}>
      <IconButton
        label={silent ? "Unmute" : "Mute"}
        tooltip={silent ? "Unmute" : "Mute"}
        icon={<Icon size={18} />}
        onClick={() => player.toggleMute()}
      />
      <VolumeReadout value={v} silent={silent} dragging={local !== null}>
        <Slider
          label="Volume"
          className="volume__slider"
          value={v}
          max={1}
          step={VOLUME_SLIDER_STEP}
          pageStep={VOLUME_SLIDER_STEP}
          valueText={(x) => `${Math.round(x * 100)}%`}
          onChange={(x) => {
            setLocal(x);
            player.volumeInput(x);
          }}
          onCommit={(x) => {
            setLocal(null);
            player.volumeCommit(x);
          }}
        />
      </VolumeReadout>
    </div>
  );
}

/** Shown while a private session is on; opens Settings. */
function PrivateSession() {
  const on = useSettings((s) => s.settings?.privateSession ?? false);
  if (!on) return null;
  return (
    <Tooltip label="Plays aren't saved to your history. Click to change." describe>
      <button type="button" className="private-pill" aria-label="Private session is on" onClick={() => nav.go({ name: "settings" })}>
        <EyeOff size={14} aria-hidden="true" />
        <span className="private-pill__label">Private session</span>
      </button>
    </Tooltip>
  );
}

/** Shuffle, previous, play/pause, next and repeat (the player bar and the full-window view). */
export function PlayControls() {
  const state = usePlayer((s) => s.state);
  const playing = state?.status === "playing" || state?.status === "buffering" || state?.status === "loading";
  const repeat = state?.repeat ?? "off";
  const shuffle = state?.shuffle ?? false;
  const smart = !!state?.smartShuffle;
  const smartAllowed = useSettings((s) => !!s.settings?.smartShuffle);
  const shuffleLabel = smart ? "Disable smart shuffle" : shuffle ? (smartAllowed ? "Enable smart shuffle" : "Disable shuffle") : "Enable shuffle";
  return (
    <div className="player-bar__controls">
      <IconButton
        label={shuffleLabel}
        tooltip={`${shuffleLabel} (${modKey}+S)`}
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
      <IconButton label="Previous" tooltip={`Previous (${modKey}+←)`} icon={<SkipBack size={20} fill="currentColor" />} onClick={() => player.previous()} />
      <IconButton
        label={playing ? "Pause" : "Play"}
        tooltip={`${playing ? "Pause" : "Play"} (Space)`}
        variant="accent"
        size="lg"
        className="player-bar__play"
        icon={playing ? <Pause size={20} fill="currentColor" /> : <Play size={20} fill="currentColor" className="nudge-right" />}
        onClick={() => player.toggle()}
      />
      <IconButton label="Next" tooltip={`Next (${modKey}+→)`} icon={<SkipForward size={20} fill="currentColor" />} onClick={() => player.next()} />
      <IconButton
        label={repeat === "off" ? "Enable repeat" : repeat === "all" ? "Enable repeat one" : "Disable repeat"}
        tooltip={`${repeat === "off" ? "Enable repeat" : repeat === "all" ? "Enable repeat one" : "Disable repeat"} (${modKey}+R)`}
        pressed={repeat !== "off"}
        icon={repeat === "one" ? <Repeat1 size={18} /> : <Repeat size={18} />}
        onClick={() => player.cycleRepeat(repeat)}
      />
    </div>
  );
}

/**
 * Opens the expanded view in lyrics mode, or leaves it when lyrics show. Off
 * with a tooltip when the song has none (but live while lyrics show, so
 * there's always a way out).
 */
function LyricsButton() {
  const trackId = usePlayer((s) => s.state?.track?.id ?? null);
  const { lyrics, loading } = useLyrics(trackId);
  const on = useFullWindow((s) => s.open && s.mode === "lyrics");
  const none = !trackId || (!loading && !lyrics);
  const off = none && !on;
  // Off, the name says why, so a screen reader hears the reason too.
  const reason = trackId ? "No lyrics for this song" : "Lyrics, nothing playing";
  return (
    <IconButton
      label={off ? reason : "Lyrics"}
      tooltip={off ? (trackId ? "No lyrics for this song" : "Lyrics") : "Lyrics"}
      pressed={on}
      softDisabled={off}
      icon={<MicVocal size={18} />}
      onClick={fullWindow.toggleLyrics}
    />
  );
}

/** Opens the expanded view in the mode last used, or closes it. */
function ExpandButton({ disabled }: { disabled: boolean }) {
  const open = useFullWindow((s) => s.open);
  return (
    <IconButton
      label={disabled && !open ? "Expanded view, nothing playing" : "Expanded view"}
      tooltip={open ? "Close expanded view" : "Expanded view"}
      pressed={open}
      disabled={disabled && !open}
      icon={<Maximize2 size={16} />}
      onClick={fullWindow.toggle}
    />
  );
}

/** Opens the miniplayer window, or closes it while it's open. */
function MiniplayerButton() {
  const open = useMiniplayer((s) => s.open);
  return (
    <IconButton
      label={open ? "Close miniplayer" : "Open miniplayer"}
      pressed={open}
      icon={<PictureInPicture2 size={16} />}
      onClick={() => void miniplayer.toggle()}
    />
  );
}

/** The queue button; songs dropped on it go to the end of the queue. */
function QueueButton() {
  const queueOpen = useUi((s) => s.queueOpen);
  const toggleQueue = useUi((s) => s.toggleQueue);
  const drop = useSongDrop("queue", "the queue", () => "ok", (songs) => dropOnQueue(songs, "end"));
  return (
    <span className={`drop-button ${drop.over ? "is-drop-target" : ""}`} {...drop.handlers}>
      <IconButton label="Queue" pressed={queueOpen} icon={<ListMusic size={18} />} onClick={toggleQueue} />
    </span>
  );
}

/**
 * A polite "Now playing: <title> by <artist>" line for screen readers when
 * the song changes (not on first load, which would talk over the app).
 */
function NowPlayingAnnouncer({ text }: { text: string | null }) {
  const [said, setSaid] = useState("");
  const first = useRef(true);
  useEffect(() => {
    if (first.current) {
      first.current = false;
      return;
    }
    setSaid(text ?? "");
  }, [text]);
  return (
    <span className="sr-only" aria-live="polite" aria-atomic="true" data-testid="now-playing-announcer">
      {said}
    </span>
  );
}

/** Bottom player bar. */
export function PlayerBar() {
  const state = usePlayer((s) => s.state);
  const nowPlayingOpen = useUi((s) => s.nowPlayingOpen);
  const outputOpen = useUi((s) => s.outputOpen);
  const toggleOutput = useUi((s) => s.toggleOutput);
  const track = state?.track;
  const songDrags = useSettings((s) => s.settings?.dragAndDrop !== false);
  // The playing song drags like a track row.
  const dragNow =
    track && songDrags
      ? { draggable: true, onDragStart: (e: React.DragEvent) => void startSongDrag(e, tracksDrag([track])), onDragEnd: endSongDrag }
      : null;

  const nowText = track ? `Now playing: ${track.title} by ${trackArtistLine(track)}` : null;

  return (
    <aside className="player-bar" aria-label="Player">
      <NowPlayingAnnouncer text={nowText} />
      <div className="player-bar__now" role="region" aria-label={nowText ?? "Nothing playing"} {...dragNow}>
        {track ? (
          <>
            <button
              type="button"
              className="player-bar__art"
              aria-label="Now playing view"
              aria-pressed={nowPlayingOpen}
              // A double-click opens the expanded view; its second click
              // mustn't toggle the panel back.
              onClick={(e) => e.detail < 2 && useUi.getState().toggleNowPlaying()}
              onDoubleClick={() => {
                useUi.getState().toggleNowPlaying();
                fullWindow.open();
              }}
            >
              <Art path={track.thumb} size={56} />
            </button>
            <div className="player-bar__meta">
              <div
                className="player-bar__title truncate"
                onContextMenu={(e) => {
                  e.preventDefault();
                  openContextMenu(`Options for ${track.title}`, trackMenuItems(track), e);
                }}
              >
                <TextLink onActivate={() => nav.go({ name: "album", id: track.albumId })}>{track.title}</TextLink>
                <ExplicitMark trackId={track.id} />
              </div>
              <PlayerSubline track={track} />
            </div>
            <SaveButton track={track} place="bar" />
          </>
        ) : state === null ? (
          // Still loading the restored session: reserve the same space (no layout shift).
          <>
            <Skeleton width={56} height={56} />
            <div className="player-bar__meta" style={{ flex: 1 }}>
              <Skeleton width="60%" height={14} />
              <Skeleton width="40%" height={12} style={{ marginTop: 6 }} />
            </div>
          </>
        ) : (
          <div className="player-bar__empty">Pick something to play</div>
        )}
      </div>

      <div className="player-bar__center">
        <PlayControls />
        <SeekBar />
      </div>

      <div className="player-bar__extras">
        <PrivateSession />
        <LyricsButton />
        <QueueButton />
        <IconButton label="Connect to a device" pressed={outputOpen} icon={<MonitorSpeaker size={18} />} onClick={toggleOutput} />
        <Volume />
        <MiniplayerButton />
        <ExpandButton disabled={!track} />
      </div>
    </aside>
  );
}
