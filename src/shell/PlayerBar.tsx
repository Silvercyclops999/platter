import {
  EyeOff,
  Heart,
  ListMusic,
  MicVocal,
  MonitorSpeaker,
  Pause,
  Play,
  Repeat,
  Repeat1,
  Shuffle,
  SkipBack,
  SkipForward,
  Volume1,
  Volume2,
  VolumeX,
} from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Art } from "../components/Art";
import { ArtistLinks, TextLink } from "../components/ArtistLinks";
import { openContextMenu } from "../components/ContextMenu";
import { trackMenuItems } from "../components/TrackRow";
import { IconButton } from "../components/Button";
import { Skeleton } from "../components/Skeleton";
import { Slider } from "../components/Slider";
import { Tooltip } from "../components/Tooltip";
import { formatDuration } from "../lib/format";
import { modKey } from "../lib/platform";
import { player, usePlayer } from "../stores/player";
import { nav } from "../stores/router";
import { useSettings } from "../stores/settings";
import { useUi } from "../stores/ui";

/** Seek bar keys: arrows move 5 s, Page Up/Down 25 s. */
export const SEEK_KEY_STEP = 5_000;
export const SEEK_PAGE_STEP = 25_000;

function SeekBar() {
  const positionMs = usePlayer((s) => s.positionMs);
  const durationMs = usePlayer((s) => s.durationMs);
  const hasTrack = usePlayer((s) => !!s.state?.track);
  const [drag, setDrag] = useState<number | null>(null);
  const shown = drag ?? positionMs;
  return (
    <div className="seek">
      <span className="seek__time tabular" aria-hidden="true">
        {formatDuration(shown)}
      </span>
      <Slider
        label="Seek"
        value={Math.min(shown, durationMs)}
        max={Math.max(durationMs, 1)}
        step={SEEK_KEY_STEP}
        pageStep={SEEK_PAGE_STEP}
        disabled={!hasTrack}
        valueText={(v) => `${formatDuration(v)} of ${formatDuration(durationMs)}`}
        // Pressing seeks at once; while held the time stays at the pointer,
        // and letting go commits where it ended.
        commitOnPress
        onChange={(v) => setDrag(v)}
        onCommit={(v, cause) => {
          if (cause !== "press") setDrag(null);
          player.seek(Math.round(v));
        }}
      />
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

function Volume() {
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

/** Bottom player bar. */
export function PlayerBar() {
  const state = usePlayer((s) => s.state);
  const queueOpen = useUi((s) => s.queueOpen);
  const toggleQueue = useUi((s) => s.toggleQueue);
  const nowPlayingOpen = useUi((s) => s.nowPlayingOpen);
  const outputOpen = useUi((s) => s.outputOpen);
  const toggleOutput = useUi((s) => s.toggleOutput);
  const track = state?.track;
  const playing = state?.status === "playing" || state?.status === "buffering" || state?.status === "loading";
  const repeat = state?.repeat ?? "off";
  const shuffle = state?.shuffle ?? false;

  return (
    <footer className="player-bar" aria-label="Player">
      <div className="player-bar__now">
        {track ? (
          <>
            <button
              type="button"
              className="player-bar__art"
              aria-label="Now playing view"
              aria-pressed={nowPlayingOpen}
              onClick={() => useUi.getState().toggleNowPlaying()}
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
              </div>
              <ArtistLinks track={track} className="player-bar__artist truncate" />
            </div>
            <IconButton label="Like" tooltip="Likes arrive in Phase 2" softDisabled icon={<Heart size={16} />} size="md" />
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
        <div className="player-bar__controls">
          <IconButton
            label={shuffle ? "Disable shuffle" : "Enable shuffle"}
            tooltip={`${shuffle ? "Disable shuffle" : "Enable shuffle"} (${modKey}+S)`}
            pressed={shuffle}
            icon={<Shuffle size={18} />}
            onClick={() => player.setShuffle(!shuffle)}
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
        <SeekBar />
      </div>

      <div className="player-bar__extras">
        <PrivateSession />
        <IconButton label="Lyrics" tooltip="Lyrics arrive in Phase 3" softDisabled icon={<MicVocal size={18} />} />
        <IconButton label="Queue" pressed={queueOpen} icon={<ListMusic size={18} />} onClick={toggleQueue} />
        <IconButton label="Connect to a device" pressed={outputOpen} icon={<MonitorSpeaker size={18} />} onClick={toggleOutput} />
        <Volume />
      </div>
    </footer>
  );
}
