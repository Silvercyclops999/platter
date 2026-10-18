/**
 * Player state mirrored from the core: `player://state` replaces the snapshot,
 * `player://position` updates position only (4×/s) so only the seek bar
 * re-renders on ticks.
 *
 * Remote mode: while playback is on another device (`remote://state` has a
 * session), `state` shows that device's song, play state, position, volume,
 * shuffle and repeat over the local snapshot, and the controls below send
 * remote commands instead of local ones.
 */
import { create } from "zustand";
import { backend, type Backend } from "../lib/backend";
import { contextShuffleMode, nextShuffleMode, playingShuffleMode, smartOffered } from "../lib/shuffleMode";
import type {
  AbLoop,
  PlayContextArgs,
  PlayContextKind,
  PlayerState,
  PlayerStatus,
  RemoteCommand,
  RemoteSession,
  RemoteState,
  RepeatMode,
  QueueAt,
} from "../lib/types";
import { OFFLINE_SKIPPED } from "../lib/offline";
import { useApp } from "./app";
import { addToSession, controlSession, guestLive } from "./social";
import { toast } from "./toasts";

interface PlayerStore {
  /** What the player bar shows: the local snapshot, or the remote device's over it. */
  state: PlayerState | null;
  /** The local player's own snapshot. */
  local: PlayerState | null;
  positionMs: number;
  durationMs: number;
  /** Playback on another device, if any. */
  remote: RemoteSession | null;
  /** The other device stopped by itself; controls are local again. */
  stoppedOn: string | null;
  set: (s: PlayerState) => void;
  setPosition: (positionMs: number, durationMs: number) => void;
  setRemote: (r: RemoteState) => void;
}

const REMOTE_STATUS: Record<RemoteSession["status"], PlayerStatus> = {
  connecting: "loading",
  playing: "playing",
  paused: "paused",
  buffering: "buffering",
};

/** The local snapshot with a remote session laid over it. */
export function withRemote(local: PlayerState | null, r: RemoteSession | null): PlayerState | null {
  if (!r || !local) return local;
  return {
    ...local,
    status: REMOTE_STATUS[r.status],
    track: r.track ?? local.track,
    positionMs: r.positionMs,
    durationMs: r.durationMs || r.track?.durationMs || local.durationMs,
    volume: r.volume ?? local.volume,
    muted: false,
    shuffle: r.shuffle,
    repeat: r.repeat,
    abLoop: null,
  };
}

export const usePlayer = create<PlayerStore>()((set, get) => ({
  state: null,
  local: null,
  positionMs: 0,
  durationMs: 0,
  remote: null,
  stoppedOn: null,
  set: (s) => {
    const r = get().remote;
    const shown = withRemote(s, r)!;
    set({ local: s, state: shown, ...(r ? {} : { positionMs: s.positionMs, durationMs: s.durationMs }) });
  },
  // Local position ticks don't move the bar while another device plays.
  setPosition: (positionMs, durationMs) => {
    if (!get().remote) set({ positionMs, durationMs });
  },
  setRemote: (rs) => {
    const local = get().local;
    const shown = withRemote(local, rs.session);
    set({
      remote: rs.session,
      stoppedOn: rs.stoppedOn,
      state: shown,
      positionMs: shown?.positionMs ?? 0,
      durationMs: shown?.durationMs ?? 0,
    });
  },
}));

/** Playing on another device now. */
export const isRemote = () => usePlayer.getState().remote !== null;

/** Subscribe the store to the core. Returns a cleanup function. */
export function connectPlayer(b: Backend = backend): () => void {
  let alive = true;
  const unsubs: Array<() => void> = [];
  b.playerState().then((s) => alive && usePlayer.getState().set(s)).catch(() => {});
  b.listen("player://state", (s) => usePlayer.getState().set(s)).then((u) => (alive ? unsubs.push(u) : u()));
  b.listen("player://position", (p) => usePlayer.getState().setPosition(p.positionMs, p.durationMs)).then((u) =>
    alive ? unsubs.push(u) : u(),
  );
  // A track that can't be played is skipped by the core; say so. While the
  // server is out of reach every queued song that isn't on this computer
  // fails in turn, so that's said once, in plain words, until it's back.
  let offlineSkipShown = false;
  unsubs.push(
    useApp.subscribe((s) => {
      if (!s.health?.cachedOnly) offlineSkipShown = false;
    }),
  );
  b.listen("player://error", (e) => {
    // A pause because the server can't be reached is said as it is.
    if (!useApp.getState().health?.cachedOnly || !e.message.startsWith("Skipped")) return toast.error(e.message);
    if (offlineSkipShown) return;
    offlineSkipShown = true;
    toast.error(OFFLINE_SKIPPED);
  }).then((u) => (alive ? unsubs.push(u) : u()));
  b.remoteState()
    .then((r) => alive && usePlayer.getState().setRemote(r))
    .catch(() => {});
  b.listen("remote://state", (r) => usePlayer.getState().setRemote(r)).then((u) => (alive ? unsubs.push(u) : u()));
  return () => {
    alive = false;
    unsubs.forEach((u) => u());
  };
}

const report = (e: unknown) => {
  const msg = e && typeof e === "object" && "message" in e ? String((e as { message: unknown }).message) : "Something went wrong";
  toast.error(msg);
};

/**
 * Volume and mute rules:
 *  - The core keeps `volume` (the remembered level) and `muted` apart, so
 *    unmute restores the exact level from before muting.
 *  - A level of 0 always counts as muted. Dragging or stepping to 0 mutes and
 *    keeps the level from before the drag as the one unmute restores.
 *  - Changing the level while muted unmutes at the new level.
 *  - Wheel/keys up while muted unmute counting up from 0.
 */

/** Last requested level (0 = muted), so fast wheel ticks build on it. */
let pendingVolume: { value: number; at: number } | null = null;
/** Level from before the current slider drag, and whether it unmuted yet. */
let drag: { from: number; unmuted: boolean } | null = null;
/** Level unmute uses when there's nothing better to restore. */
export const UNMUTE_FALLBACK = 0.5;

/** Forget in-flight volume nudges and drags (tests, or after a slider drag). */
export function resetVolumeNudge() {
  pendingVolume = null;
  drag = null;
}

/** Send a control to the device playing now; its answer updates the bar at once. */
const remoteCmd = (c: RemoteCommand) => backend.remoteCommand(c).then((r) => usePlayer.getState().setRemote(r));

/** Level to restore after muting a remote device (players have no mute). */
let remoteUnmuteTo = 0.5;

/** Volume and mute go to the device playing now. */
const out = {
  setVolume: (v: number): Promise<unknown> => (isRemote() ? remoteCmd({ kind: "volume", volume: v }) : backend.setVolume(v)),
  setMuted: (m: boolean): Promise<unknown> => {
    if (!isRemote()) return backend.setMuted(m);
    const cur = usePlayer.getState().remote?.volume ?? 0;
    if (m) {
      if (cur > 0) remoteUnmuteTo = cur;
      return remoteCmd({ kind: "volume", volume: 0 });
    }
    return cur > 0 ? Promise.resolve() : remoteCmd({ kind: "volume", volume: remoteUnmuteTo });
  },
};

/**
 * Listening along, the queue here mirrors the host's: a remove or clear
 * here would come back with the next update, so say so instead.
 */
const hostOnly = (): Promise<void> => {
  toast.info("Only the host can take songs out of the session's queue");
  return Promise.resolve();
};

/** Start a context here, or on the device playing now. */
const play = (args: PlayContextArgs): Promise<unknown> =>
  isRemote() ? backend.remotePlayContext(args).then((r) => usePlayer.getState().setRemote(r)) : backend.playContext(args);

const round = (v: number) => Math.round(Math.min(1, Math.max(0, v)) * 10_000) / 10_000;

/** The toast after songs go to the end of the queue. */
export function queuedToast(n: number, thumb?: string | null) {
  toast.info(n === 1 ? "Added to queue" : `Added ${n} songs to queue`, thumb ? { path: thumb } : undefined);
}

/** Player commands with error toasts. */
export const player = {
  toggle: () => {
    // Listening along: the controls go to the host, for everyone.
    if (guestLive()) return controlSession({ op: usePlayer.getState().state?.status === "playing" ? "pause" : "play" });
    if (isRemote()) {
      const playing = usePlayer.getState().state?.status === "playing";
      return remoteCmd({ kind: playing ? "pause" : "play" }).catch(report);
    }
    return backend.playToggle().catch(report);
  },
  /** Pause (no toggle): the record player's stop at the end of a side. */
  pause: () => (isRemote() ? remoteCmd({ kind: "pause" }) : backend.playPause()).catch(report),
  /** Play on from a pause (no toggle). */
  resume: () => (isRemote() ? remoteCmd({ kind: "play" }) : backend.playResume()).catch(report),
  next: () => (guestLive() ? controlSession({ op: "next" }) : (isRemote() ? remoteCmd({ kind: "next" }) : backend.playNext()).catch(report)),
  previous: () =>
    guestLive() ? controlSession({ op: "previous" }) : (isRemote() ? remoteCmd({ kind: "previous" }) : backend.playPrevious()).catch(report),
  seek: (ms: number) => {
    if (guestLive()) return controlSession({ op: "seek", ms });
    if (isRemote()) {
      usePlayer.setState({ positionMs: ms });
      return remoteCmd({ kind: "seek", positionMs: ms }).catch(report);
    }
    usePlayer.getState().setPosition(ms, usePlayer.getState().durationMs);
    return backend.playSeek(ms).catch(report);
  },
  /** Seek relative to the current position, clamped to the track. */
  seekBy: (deltaMs: number): Promise<unknown> => {
    const { state, positionMs, durationMs } = usePlayer.getState();
    if (!state?.track) return Promise.resolve();
    const to = Math.round(Math.min(Math.max(0, positionMs + deltaMs), durationMs));
    return player.seek(to);
  },
  setVolume: (v: number) => out.setVolume(v).catch(report),
  setMuted: (m: boolean) => out.setMuted(m).catch(report),
  setShuffle: (s: boolean) => (isRemote() ? remoteCmd({ kind: "shuffle", on: s }) : backend.setShuffle(s)).catch(report),
  setContextShuffle: (id: string, s: boolean) => backend.setContextShuffle(id, s).catch(report),
  /** Set or clear the A-B loop. Shows at once; a refusal puts the old one back. */
  setAbLoop: (l: AbLoop | null) => {
    const before = usePlayer.getState().state;
    const shown = before ? { ...before, abLoop: l } : null;
    if (shown) usePlayer.setState({ state: shown });
    return backend.setAbLoop(l).catch((e) => {
      if (before && usePlayer.getState().state === shown) usePlayer.setState({ state: before });
      report(e);
    });
  },
  cycleRepeat: (r: RepeatMode) => {
    const mode: RepeatMode = r === "off" ? "all" : r === "all" ? "one" : "off";
    return (isRemote() ? remoteCmd({ kind: "repeat", mode }) : backend.setRepeat(mode)).catch(report);
  },
  playAlbum: (id: string, startTrackId?: string, shuffle?: boolean) =>
    play({ context: { kind: "album", id }, startTrackId, shuffle }).catch(report),
  /** One side of a record: only its songs, still playing from the album. */
  playSide: (albumId: string, trackIds: string[]) => play({ context: { kind: "album", id: albumId, trackIds } }).catch(report),
  /** A playlist; `trackIds` plays it in the page's order (a sort). */
  playPlaylist: (id: string, startTrackId?: string, shuffle?: boolean, trackIds?: string[], startIndex?: number) =>
    play({ context: { kind: "playlist", id, trackIds }, startTrackId, startIndex, shuffle }).catch(report),
  /** Liked Songs; `trackIds` plays them in the page's order (a sort or filter). */
  playLiked: (startTrackId?: string, trackIds?: string[], shuffle?: boolean) =>
    play({ context: { kind: "liked", id: "liked", trackIds }, startTrackId, shuffle }).catch(report),
  playArtist: (id: string, startTrackId?: string, shuffle?: boolean) =>
    play({ context: { kind: "artist", id }, startTrackId, shuffle }).catch(report),
  /** A made-for-you mix; `trackIds` plays a steered mix as shown. */
  playMix: (id: string, trackIds?: string[], startTrackId?: string) =>
    play({ context: { kind: "mix", id, trackIds }, startTrackId }).catch(report),
  /** A radio (`track:<id>` or `artist:<id>`): the page's songs, then more like them. */
  playRadio: (seed: string, trackIds?: string[], startTrackId?: string) =>
    play({ context: { kind: "radio", id: seed, trackIds }, startTrackId }).catch(report),
  setSmartShuffle: (on: boolean) => backend.setSmartShuffle(on).catch(report),
  setSmoothTransitions: (on: boolean) => backend.setSmoothTransitions(on).catch(report),
  playTracks: (trackIds: string[], startTrackId?: string, kind: "tracks" | "search" = "tracks") =>
    play({ context: { kind, trackIds }, startTrackId }).catch(report),
  queueAdd: (trackIds: string[], at: QueueAt) =>
    // Listening along: the queue here is the host's, so add there.
    guestLive() ? addToSession(trackIds) : backend.queueAdd(trackIds, at).catch(report),
  /**
   * "Add to queue" from a menu: to the end of the queue, then a short toast
   * with the cover (the song's own for one song, else `thumb` when given).
   */
  addToQueue: (tracks: { id: string; thumb: string | null }[], thumb?: string | null) => {
    if (!tracks.length) return Promise.resolve();
    // Listening along: into the shared queue, through the host.
    if (guestLive()) return addToSession(tracks.map((t) => t.id));
    return backend
      .queueAdd(
        tracks.map((t) => t.id),
        "end",
      )
      .then(() => queuedToast(tracks.length, tracks.length === 1 ? tracks[0].thumb : thumb), report);
  },
  queueAlbum: (id: string) =>
    backend
      .albumPage(id)
      .then((p) => player.addToQueue(p.tracks, p.album.thumb))
      .catch(report),
  queuePlaylist: (id: string) =>
    backend
      .playlistPage(id)
      .then((p) => player.addToQueue(p.tracks, p.playlist.thumb))
      .catch(report),
  /**
   * Nudge the volume (keyboard, mouse wheel). Raising it while muted unmutes
   * counting up from 0; lowering it to 0 mutes and keeps the last level.
   */
  nudgeVolume: async (delta: number) => {
    const s = usePlayer.getState().state;
    if (!s) return;
    // Fast wheel ticks / key repeats build on the last requested value, not
    // the (slightly stale) one the core has echoed back.
    const now = Date.now();
    const fresh = pendingVolume && now - pendingVolume.at < 500;
    const base = fresh ? pendingVolume!.value : s.muted || s.volume <= 0 ? 0 : s.volume;
    const v = round(base + delta);
    pendingVolume = { value: v, at: now };
    if (v === base) return;
    try {
      if (v === 0) {
        // Stepped down to 0: mute; the core keeps `base` as the level to restore.
        await out.setMuted(true);
      } else if (base === 0) {
        // Muted: set the new low level first so unmuting doesn't jump to the old one.
        await out.setVolume(v);
        await out.setMuted(false);
      } else {
        await out.setVolume(v);
      }
    } catch (e) {
      report(e);
    }
  },
  /** Speaker button: mute, or unmute to the remembered level. */
  toggleMute: async () => {
    const s = usePlayer.getState().state;
    if (!s) return;
    pendingVolume = null;
    // Players have no mute: volume 0, then back to the level from before.
    if (isRemote()) return out.setMuted(s.volume > 0).catch(report);
    try {
      if (s.muted || s.volume <= 0) {
        if (s.volume <= 0) await out.setVolume(UNMUTE_FALLBACK);
        await out.setMuted(false);
      } else {
        await out.setMuted(true);
      }
    } catch (e) {
      report(e);
    }
  },
  /** Volume slider moved (press, drag or key). Unmutes as soon as the level is above 0. */
  volumeInput: (v: number) => {
    const s = usePlayer.getState().state;
    if (!s) return Promise.resolve();
    pendingVolume = null;
    if (!drag) drag = { from: s.volume, unmuted: false };
    if (v <= 0) return out.setVolume(0).catch(report);
    const unmute = s.muted && !drag.unmuted;
    drag.unmuted ||= unmute;
    return out
      .setVolume(v)
      .then(() => (unmute ? out.setMuted(false) : undefined))
      .catch(report);
  },
  /**
   * Volume slider let go (or a key step). Ending at 0 mutes and remembers the
   * level from before the drag; anything else unmutes at the new level.
   */
  volumeCommit: async (v: number) => {
    const s = usePlayer.getState().state;
    const from = drag?.from ?? s?.volume ?? UNMUTE_FALLBACK;
    drag = null;
    pendingVolume = null;
    if (!s) return;
    try {
      if (v <= 0) {
        // Mute first so restoring the old level is never audible.
        await out.setMuted(true);
        await out.setVolume(from > 0 ? from : UNMUTE_FALLBACK);
      } else {
        await out.setVolume(v);
        if (s.muted) await out.setMuted(false);
      }
    } catch (e) {
      report(e);
    }
  },
  /**
   * The player bar's shuffle button: off, shuffle, then smart shuffle on a
   * playlist or Liked Songs while its setting is on. Smart shuffle off turns
   * shuffle off too (the order stays as shuffled).
   */
  toggleShuffle: (smartAllowed = false) => {
    const s = usePlayer.getState().state;
    const mode = playingShuffleMode(s);
    const ctx = s?.queue.context;
    const smart = smartAllowed && !isRemote() && smartOffered(ctx?.kind) && !!ctx?.id;
    if (mode === "smart") return isRemote() ? player.setShuffle(false) : player.setSmartShuffle(false);
    const next = nextShuffleMode(mode, smart);
    return next === "smart" ? player.setSmartShuffle(true) : player.setShuffle(next === "shuffle");
  },
  /** A page's shuffle toggle: the same cycle for a context by id, playing or not. */
  cycleContextShuffle: (id: string, kind: PlayContextKind, smartAllowed = false) => {
    const mode = contextShuffleMode(usePlayer.getState().state, id);
    if (mode === "smart") return backend.setContextSmartShuffle(id, false).catch(report);
    const next = nextShuffleMode(mode, smartAllowed && smartOffered(kind));
    return next === "smart" ? backend.setContextSmartShuffle(id, true).catch(report) : player.setContextShuffle(id, next === "shuffle");
  },
  cycleRepeatNow: () => player.cycleRepeat(usePlayer.getState().state?.repeat ?? "off"),
  queueRemove: (itemId: string) => (guestLive() ? hostOnly() : backend.queueRemove(itemId).catch(report)),
  queueMove: (itemId: string, toIndex: number) => {
    // Listening along: the queue here mirrors the host's, so move it there.
    if (guestLive()) {
      const q = usePlayer.getState().state?.queue;
      const from = q ? [...q.userQueue, ...q.nextUp].findIndex((i) => i.itemId === itemId) : -1;
      return from < 0 ? Promise.resolve() : controlSession({ op: "move", from, to: toIndex });
    }
    return backend.queueMove(itemId, toIndex).catch(report);
  },
  queueClear: () => (guestLive() ? hostOnly() : backend.queueClear().catch(report)),
  queueJump: (itemId: string) =>
    (isRemote()
      ? backend.remoteQueueJump(itemId).then((r) => usePlayer.getState().setRemote(r))
      : backend.queueJump(itemId)
    ).catch(report),
};
