import {
  CircleMinus,
  CirclePlus,
  EyeOff,
  ListMusic,
  Maximize2,
  MicVocal,
  MonitorSpeaker,
  MoreHorizontal,
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
import { Fragment, useEffect, useRef, useState } from "react";
import { Art } from "../components/Art";
import { TextLink } from "../components/ArtistLinks";
import { ListenMark } from "./ListenStrip";
import { ExplicitMark } from "../components/FileTagBadges";
import { openContextMenu } from "../components/ContextMenu";
import { SaveButton } from "../components/SaveButton";
import { trackMenuItems } from "../components/TrackRow";
import { IconButton } from "../components/Button";
import { Skeleton } from "../components/Skeleton";
import { Slider } from "../components/Slider";
import { Tooltip } from "../components/Tooltip";
import { formatDuration, trackArtistLine } from "../lib/format";
import { playingShuffleMode, shuffleLabel as labelFor, smartOffered } from "../lib/shuffleMode";
import { modKey } from "../lib/platform";
import { dropOnQueue, endSongDrag, startSongDrag, tracksDrag, useSongDrop } from "../stores/drag";
import { player, usePlayer } from "../stores/player";
import { fullWindow, useFullWindow } from "../stores/fullWindow";
import { miniplayer, useMiniplayer } from "../stores/miniplayer";
import { nav } from "../stores/router";
import { useSettings } from "../stores/settings";
import { useLayout } from "../stores/theme";
import { keepSuggestion, playingSuggestion, removeSuggestion } from "../stores/smartShuffle";
import { useUi } from "../stores/ui";
import { LoopChip, LoopLayer, PlayerSubline, VolumeReadout } from "./PlayerExtras";
import { actionableNotice, useLyrics } from "./lyrics";
import { normalizeLoop } from "./playerRules";
import { HifiMark } from "./HifiMark";
import { SignalBadge, SignalPanel } from "./SignalPath";
import { openSignal } from "../stores/signalPath";
import { AudioLines } from "lucide-react";

/** Seek bar keys: arrows move 5 s, Page Up/Down 25 s. */
export const SEEK_KEY_STEP = 5_000;
export const SEEK_PAGE_STEP = 25_000;

/**
 * The seek line with its times and format plate. `strip` is the transport
 * strip's version: always the tonearm line, times in engraved plates.
 */
export function SeekBar({ strip = false }: { strip?: boolean } = {}) {
  const positionMs = usePlayer((s) => s.positionMs);
  const durationMs = usePlayer((s) => s.durationMs);
  const trackId = usePlayer((s) => s.state?.track?.id ?? null);
  const hasTrack = trackId !== null;
  const loopOn = useSettings((s) => s.settings?.abLoopHandles ?? true);
  const tonearm = useSettings((s) => s.settings?.tonearmSeek ?? false) || strip;
  const loop = usePlayer((s) => s.state?.abLoop ?? null);
  // The time held while dragging, for the song it was dragged on.
  const [held, setHeld] = useState<{ v: number; id: string | null } | null>(null);
  const drag = held && held.id === trackId ? held.v : null;
  const setDrag = (v: number | null) => setHeld(v === null ? null : { v, id: trackId });
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
    <div className={strip ? "seek seek--strip" : "seek"}>
      {showLoop && loop && <LoopChip onClear={clearLoop} />}
      <span className="seek__time tabular" aria-hidden="true">
        {formatDuration(shown)}
      </span>
      <div className="seek__bar">
        <Slider
          label={hasTrack ? "Seek" : "Seek, nothing playing"}
          className={tonearm ? "slider--tonearm" : ""}
          value={Math.min(shown, durationMs)}
          max={Math.max(durationMs, 1)}
          step={SEEK_KEY_STEP}
          pageStep={SEEK_PAGE_STEP}
          disabled={!hasTrack}
          resetKey={trackId}
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
      {/* The format sits on the time line, not floating in the right group. */}
      <span className="player-bar__fold">
        <SignalBadge />
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
export function PrivateSession() {
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
  const remote = usePlayer((s) => !!s.remote);
  // Playlists and Liked Songs cycle through smart shuffle too, and the name
  // says what the next click does for that playlist.
  const ctx = state?.queue.context;
  const offered = smartAllowed && !remote && smartOffered(ctx?.kind) && !!ctx?.id;
  const shuffleLabel = labelFor(playingShuffleMode(state), offered || smart, ctx?.kind === "liked" ? "Liked Songs" : ctx?.title);
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
export function LyricsButton() {
  const trackId = usePlayer((s) => s.state?.track?.id ?? null);
  const { lyrics, loading, info } = useLyrics(trackId);
  const on = useFullWindow((s) => s.open && s.mode === "lyrics");
  const none = !trackId || (!loading && !lyrics && !actionableNotice(info));
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
export function ExpandButton({ disabled }: { disabled: boolean }) {
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
export function MiniplayerButton() {
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

/**
 * On a narrow window the miniplayer and expanded view buttons fold into this
 * menu (the CSS shows one or the other), so the right side never runs over
 * the play controls.
 */
export function MoreButton({ buttons, hasTrack }: { buttons: readonly string[]; hasTrack: boolean }) {
  const ref = useRef<HTMLSpanElement>(null);
  const folds = buttons.filter((b) => b === "miniplayer" || b === "fullWindow");
  if (!folds.length) return null;
  const open = () => {
    const folded = folds.map((b) =>
      b === "miniplayer"
        ? { label: useMiniplayer.getState().open ? "Close miniplayer" : "Open miniplayer", icon: PictureInPicture2, onSelect: () => void miniplayer.toggle() }
        : {
            label: useFullWindow.getState().open ? "Close expanded view" : "Expanded view",
            icon: Maximize2,
            disabled: !hasTrack && !useFullWindow.getState().open,
            onSelect: fullWindow.toggle,
          },
    );
    const items = hasTrack ? [...folded, { label: "Signal path", icon: AudioLines, onSelect: () => openSignal() }] : folded;
    const btn = ref.current?.querySelector("button");
    if (btn) openContextMenu("More player options", items, btn);
  };
  return (
    <span className="player-bar__more" ref={ref}>
      <IconButton label="More player options" icon={<MoreHorizontal size={18} />} onClick={open} />
    </span>
  );
}

/** The queue button; songs dropped on it go to the end of the queue. */
export function QueueButton() {
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
export function NowPlayingAnnouncer({ text }: { text: string | null }) {
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

/**
 * While a smart shuffle suggestion plays: take it out (skips it and picks new
 * suggestions) or add it to the playlist for good.
 */
export function SuggestionButtons() {
  const state = usePlayer((s) => s.state);
  const remote = usePlayer((s) => !!s.remote);
  const s = remote ? null : playingSuggestion(state);
  if (!s) return null;
  return (
    <span className="player-bar__suggestion">
      <IconButton label="Remove suggestion" tooltip="Remove suggestion" icon={<CircleMinus size={16} />} onClick={() => void removeSuggestion()} />
      <IconButton
        label={`Add to ${s.title}`}
        tooltip={`Add to ${s.title}`}
        icon={<CirclePlus size={16} />}
        onClick={() => void keepSuggestion(state)}
      />
    </span>
  );
}

/** Bottom player bar. */
export function PlayerBar() {
  const state = usePlayer((s) => s.state);
  // Over the expanded view the panel is hidden, so the cover button isn't pressed.
  const expanded = useFullWindow((s) => s.open);
  const nowPlayingOpen = useUi((s) => s.nowPlayingOpen) && !expanded;
  const outputOpen = useUi((s) => s.outputOpen);
  const toggleOutput = useUi((s) => s.toggleOutput);
  const buttons = useLayout().playerButtons;
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
              <Art path={track.thumb} sleeve={{ title: track.albumTitle, artist: track.artistTitle }} size={56} />
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
              <ListenMark />
            </div>
            <SuggestionButtons />
            {buttons.includes("like") && <SaveButton track={track} place="bar" />}
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
        {/* The Layout option picks which buttons show, and their order. The
            hi-fi mark stays beside the device button. */}
        {!buttons.includes("devices") && <HifiMark />}
        {buttons.map((id) => (
          <Fragment key={id}>
            {id === "lyrics" ? (
              <LyricsButton />
            ) : id === "queue" ? (
              <QueueButton />
            ) : id === "devices" ? (
              <>
                <HifiMark />
                <IconButton label="Connect to a device" pressed={outputOpen} icon={<MonitorSpeaker size={18} />} onClick={toggleOutput} />
              </>
            ) : id === "volume" ? (
              <Volume />
            ) : id === "miniplayer" ? (
              <span className="player-bar__fold">
                <MiniplayerButton />
              </span>
            ) : id === "fullWindow" ? (
              <span className="player-bar__fold">
                <ExpandButton disabled={!track} />
              </span>
            ) : null}
          </Fragment>
        ))}
        <MoreButton buttons={buttons} hasTrack={!!track} />
      </div>
      <SignalPanel />
    </aside>
  );
}
