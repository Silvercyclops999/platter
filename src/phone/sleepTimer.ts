/**
 * The sleep timer: playback fades out and pauses after a number of minutes,
 * at the end of the song playing, or at the end of the album playing. It
 * runs here in the UI for now; the volume comes back up after the pause so
 * the next play starts at the level it had.
 */
import { create } from "zustand";
import { player, usePlayer } from "../stores/player";

export type SleepChoice = { kind: "minutes"; minutes: number } | { kind: "track" } | { kind: "album" };

export const SLEEP_CHOICES: { label: string; choice: SleepChoice }[] = [
  { label: "5 minutes", choice: { kind: "minutes", minutes: 5 } },
  { label: "10 minutes", choice: { kind: "minutes", minutes: 10 } },
  { label: "15 minutes", choice: { kind: "minutes", minutes: 15 } },
  { label: "30 minutes", choice: { kind: "minutes", minutes: 30 } },
  { label: "45 minutes", choice: { kind: "minutes", minutes: 45 } },
  { label: "1 hour", choice: { kind: "minutes", minutes: 60 } },
  { label: "End of this song", choice: { kind: "track" } },
  { label: "End of this album", choice: { kind: "album" } },
];

/** Custom lengths, in minutes. */
export const CUSTOM_MIN = 1;
export const CUSTOM_MAX = 12 * 60;
/** How long the fade at the end takes. */
export const FADE_MS = 6_000;
/** A song counts as ended this close to its end. */
const END_SLACK_MS = 350;

export type SleepState =
  | null
  | { kind: "minutes"; endsAt: number; minutes: number }
  | { kind: "track"; trackId: string }
  | { kind: "album"; albumId: string };

interface SleepStore {
  timer: SleepState;
  /** Fading out now. */
  fading: boolean;
}

export const useSleep = create<SleepStore>()(() => ({ timer: null, fading: false }));

let clock: ReturnType<typeof setInterval> | null = null;
let fade: ReturnType<typeof setInterval> | null = null;
/** The level a fade started from, put back if the timer is turned off mid-fade. */
let fadeFrom: number | null = null;

function stopClock() {
  if (clock) clearInterval(clock);
  clock = null;
}

/** Whether the timer is due now. */
export function due(t: SleepState, now: number): boolean {
  if (!t) return false;
  const { state, positionMs, durationMs } = usePlayer.getState();
  const track = state?.track;
  if (t.kind === "minutes") return now >= t.endsAt - FADE_MS;
  if (!track) return false;
  const left = durationMs - positionMs;
  if (t.kind === "track") return track.id !== t.trackId || left <= END_SLACK_MS + FADE_MS;
  // The album: its last song, near its end (or something else started).
  if (track.albumId !== t.albumId) return true;
  const next = state?.queue.userQueue[0] ?? state?.queue.nextUp[0];
  const last = !next || next.track.albumId !== t.albumId;
  return last && left <= END_SLACK_MS + FADE_MS;
}

function fadeOut() {
  const s = usePlayer.getState().state;
  stopClock();
  if (!s || s.status !== "playing") return finish(null);
  const from = s.volume;
  fadeFrom = from;
  const start = Date.now();
  useSleep.setState({ fading: true });
  fade = setInterval(() => {
    const k = Math.min(1, (Date.now() - start) / FADE_MS);
    void player.setVolume(Math.max(0, from * (1 - k)));
    if (k >= 1) finish(from);
  }, 200);
}

function finish(restore: number | null) {
  if (fade) clearInterval(fade);
  fade = null;
  fadeFrom = null;
  useSleep.setState({ timer: null, fading: false });
  void Promise.resolve(player.pause()).then(() => (restore === null ? undefined : player.setVolume(restore)));
}

/** Start (or replace) the timer. */
export function startSleep(c: SleepChoice, now = Date.now()) {
  cancelSleep();
  const s = usePlayer.getState().state;
  let timer: SleepState = null;
  if (c.kind === "minutes") timer = { kind: "minutes", minutes: c.minutes, endsAt: now + c.minutes * 60_000 };
  else if (c.kind === "track" && s?.track) timer = { kind: "track", trackId: s.track.id };
  else if (c.kind === "album" && s?.track) timer = { kind: "album", albumId: s.track.albumId };
  if (!timer) return;
  useSleep.setState({ timer });
  clock = setInterval(() => {
    if (due(useSleep.getState().timer, Date.now())) fadeOut();
  }, 250);
}

export function cancelSleep() {
  stopClock();
  if (fade) clearInterval(fade);
  fade = null;
  // Turned off mid-fade: the music keeps playing at its own level.
  if (fadeFrom !== null) void player.setVolume(fadeFrom);
  fadeFrom = null;
  useSleep.setState({ timer: null, fading: false });
}

/** The pill's line: what the timer is set to. */
export function sleepLabel(t: SleepState, now = Date.now()): string {
  if (!t) return "Sleep timer";
  if (t.kind === "track") return "End of song";
  if (t.kind === "album") return "End of album";
  const left = Math.max(0, Math.ceil((t.endsAt - now) / 60_000));
  return left >= 60 ? `${Math.floor(left / 60)} h ${left % 60 ? `${left % 60} min` : ""}`.trim() : `${left} min`;
}
