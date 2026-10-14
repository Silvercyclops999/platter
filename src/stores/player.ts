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

let pendingVolume: { value: number; at: number } | null = null;
/** Forget in-flight volume nudges (tests, or after a slider drag). */
export function resetVolumeNudge() {
  pendingVolume = null;
}

/** Player commands with error toasts. */
export const player = {
  toggle: () => backend.playToggle().catch(report),
  next: () => backend.playNext().catch(report),
  previous: () => backend.playPrevious().catch(report),
  seek: (ms: number) => {
    usePlayer.getState().setPosition(ms, usePlayer.getState().durationMs);
    return backend.playSeek(ms).catch(report);
  },
  setVolume: (v: number) => backend.setVolume(v).catch(report),
  setMuted: (m: boolean) => backend.setMuted(m).catch(report),
  setShuffle: (s: boolean) => backend.setShuffle(s).catch(report),
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
  /** Nudge the volume (keyboard, mouse wheel). Unmutes when raising it. */
  nudgeVolume: (delta: number) => {
    const s = usePlayer.getState().state;
    if (!s) return Promise.resolve();
    // While muted, raising unmutes back to the remembered level; lowering does nothing.
    if (s.muted) return delta > 0 ? backend.setMuted(false).catch(report) : Promise.resolve();
    // Fast wheel ticks / key repeats build on the last requested value, not
    // the (slightly stale) one the core has echoed back.
    const now = Date.now();
    const base = pendingVolume && now - pendingVolume.at < 500 ? pendingVolume.value : s.volume;
    const v = Math.round(Math.min(1, Math.max(0, base + delta)) * 100) / 100;
    pendingVolume = { value: v, at: now };
    return v === base ? Promise.resolve() : backend.setVolume(v).catch(report);
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
