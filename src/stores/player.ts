/**
 * Player state mirrored from the core: `player://state` replaces the snapshot,
 * `player://position` updates position only (4×/s) so only the seek bar
 * re-renders on ticks.
 */
import { create } from "zustand";
import { backend, type Backend } from "../lib/backend";
import type { PlayerState, RepeatMode } from "../lib/types";
import { toast } from "./toasts";

interface PlayerStore {
  state: PlayerState | null;
  positionMs: number;
  durationMs: number;
  set: (s: PlayerState) => void;
  setPosition: (positionMs: number, durationMs: number) => void;
}

export const usePlayer = create<PlayerStore>()((set) => ({
  state: null,
  positionMs: 0,
  durationMs: 0,
  set: (s) => set({ state: s, positionMs: s.positionMs, durationMs: s.durationMs }),
  setPosition: (positionMs, durationMs) => set({ positionMs, durationMs }),
}));

/** Subscribe the store to the core. Returns a cleanup function. */
export function connectPlayer(b: Backend = backend): () => void {
  let alive = true;
  const unsubs: Array<() => void> = [];
  b.playerState().then((s) => alive && usePlayer.getState().set(s)).catch(() => {});
  b.listen("player://state", (s) => usePlayer.getState().set(s)).then((u) => (alive ? unsubs.push(u) : u()));
  b.listen("player://position", (p) => usePlayer.getState().setPosition(p.positionMs, p.durationMs)).then((u) =>
    alive ? unsubs.push(u) : u(),
  );
  // A track that can't be played is skipped by the core; say so.
  b.listen("player://error", (e) => toast.error(e.message)).then((u) => (alive ? unsubs.push(u) : u()));
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

const round = (v: number) => Math.round(Math.min(1, Math.max(0, v)) * 10_000) / 10_000;

/** Player commands with error toasts. */
export const player = {
  toggle: () => backend.playToggle().catch(report),
  next: () => backend.playNext().catch(report),
  previous: () => backend.playPrevious().catch(report),
  seek: (ms: number) => {
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
  setVolume: (v: number) => backend.setVolume(v).catch(report),
  setMuted: (m: boolean) => backend.setMuted(m).catch(report),
  setShuffle: (s: boolean) => backend.setShuffle(s).catch(report),
  setContextShuffle: (id: string, s: boolean) => backend.setContextShuffle(id, s).catch(report),
  cycleRepeat: (r: RepeatMode) => backend.setRepeat(r === "off" ? "all" : r === "all" ? "one" : "off").catch(report),
  playAlbum: (id: string, startTrackId?: string, shuffle?: boolean) =>
    backend.playContext({ context: { kind: "album", id }, startTrackId, shuffle }).catch(report),
  playPlaylist: (id: string, startTrackId?: string, shuffle?: boolean) =>
    backend.playContext({ context: { kind: "playlist", id }, startTrackId, shuffle }).catch(report),
  playArtist: (id: string, startTrackId?: string, shuffle?: boolean) =>
    backend.playContext({ context: { kind: "artist", id }, startTrackId, shuffle }).catch(report),
  playTracks: (trackIds: string[], startTrackId?: string, kind: "tracks" | "search" = "tracks") =>
    backend.playContext({ context: { kind, trackIds }, startTrackId }).catch(report),
  queueAdd: (trackIds: string[], at: "next" | "end") => backend.queueAdd(trackIds, at).catch(report),
  queueAlbum: (id: string) =>
    backend
      .albumPage(id)
      .then((p) => backend.queueAdd(p.tracks.map((t) => t.id), "end"))
      .catch(report),
  queuePlaylist: (id: string) =>
    backend
      .playlistPage(id)
      .then((p) => backend.queueAdd(p.tracks.map((t) => t.id), "end"))
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
        await backend.setMuted(true);
      } else if (base === 0) {
        // Muted: set the new low level first so unmuting doesn't jump to the old one.
        await backend.setVolume(v);
        await backend.setMuted(false);
      } else {
        await backend.setVolume(v);
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
    try {
      if (s.muted || s.volume <= 0) {
        if (s.volume <= 0) await backend.setVolume(UNMUTE_FALLBACK);
        await backend.setMuted(false);
      } else {
        await backend.setMuted(true);
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
    if (v <= 0) return backend.setVolume(0).catch(report);
    const unmute = s.muted && !drag.unmuted;
    drag.unmuted ||= unmute;
    return backend
      .setVolume(v)
      .then(() => (unmute ? backend.setMuted(false) : undefined))
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
        await backend.setMuted(true);
        await backend.setVolume(from > 0 ? from : UNMUTE_FALLBACK);
      } else {
        await backend.setVolume(v);
        if (s.muted) await backend.setMuted(false);
      }
    } catch (e) {
      report(e);
    }
  },
  toggleShuffle: () => backend.setShuffle(!(usePlayer.getState().state?.shuffle ?? false)).catch(report),
  cycleRepeatNow: () => {
    const r = usePlayer.getState().state?.repeat ?? "off";
    return backend.setRepeat(r === "off" ? "all" : r === "all" ? "one" : "off").catch(report);
  },
  queueRemove: (itemId: string) => backend.queueRemove(itemId).catch(report),
  queueMove: (itemId: string, toIndex: number) => backend.queueMove(itemId, toIndex).catch(report),
  queueClear: () => backend.queueClear().catch(report),
  queueJump: (itemId: string) => backend.queueJump(itemId).catch(report),
};
