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
import { useState } from "react";
import { Art } from "../components/Art";
import { IconButton } from "../components/Button";
import { Skeleton } from "../components/Skeleton";
import { Slider } from "../components/Slider";
import { Tooltip } from "../components/Tooltip";
import { formatDuration, trackArtistLine } from "../lib/format";
import { modKey } from "../lib/platform";
import { player, usePlayer } from "../stores/player";
import { nav } from "../stores/router";
import { useSettings } from "../stores/settings";
import { useUi } from "../stores/ui";

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
        step={5000}
        disabled={!hasTrack}
        valueText={(v) => `${formatDuration(v)} of ${formatDuration(durationMs)}`}
        onChange={(v) => setDrag(v)}
        onCommit={(v) => {
          setDrag(null);
          player.seek(Math.round(v));
        }}
      />
      <span className="seek__time seek__time--end tabular" aria-hidden="true">
        -{formatDuration(Math.max(0, durationMs - shown))}
      </span>
    </div>
  );
}

function Volume() {
  const volume = usePlayer((s) => s.state?.volume ?? 0.7);
  const muted = usePlayer((s) => s.state?.muted ?? false);
  const [local, setLocal] = useState<number | null>(null);
  const v = local ?? (muted ? 0 : volume);
  const Icon = muted || v === 0 ? VolumeX : v < 0.5 ? Volume1 : Volume2;
  return (
    <div className="volume">
      <IconButton
        label={muted ? "Unmute" : "Mute"}
        icon={<Icon size={18} />}
        onClick={() => player.setMuted(!muted)}
      />
      <Slider
        label="Volume"
        className="volume__slider"
        value={v}
        max={1}
        step={0.05}
        valueText={(x) => `${Math.round(x * 100)}%`}
        onChange={(x) => {
          setLocal(x);
          player.setVolume(x);
        }}
        onCommit={(x) => {
          setLocal(null);
          player.setVolume(x);
          if (x > 0 && muted) player.setMuted(false);
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
              aria-label={`Open album ${track.albumTitle}`}
              onClick={() => nav.go({ name: "album", id: track.albumId })}
            >
              <Art path={track.thumb} size={56} />
            </button>
            <div className="player-bar__meta">
              <button type="button" className="link player-bar__title truncate" onClick={() => nav.go({ name: "album", id: track.albumId })}>
                {track.title}
              </button>
              <button type="button" className="link player-bar__artist truncate" onClick={() => nav.go({ name: "artist", id: track.artistId })}>
                {trackArtistLine(track)}
              </button>
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
        <IconButton label="Connect to a device" tooltip="Devices arrive in Phase 3" softDisabled icon={<MonitorSpeaker size={18} />} />
        <Volume />
      </div>
    </footer>
  );
}
