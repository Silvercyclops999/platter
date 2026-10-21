/**
 * The sleep timer: playback fades out and pauses after a number of minutes,
 * at the end of the song playing, or at the end of the album playing. The
 * core keeps the time (so it runs with the window hidden or the phone
 * locked); this mirrors it for the player bar, the menu and the phone sheet.
 */
import { create } from "zustand";
import { backend, type Backend } from "../lib/backend";
import type { SleepChoice, SleepTimerView } from "../lib/types";
import { toast } from "./toasts";

export type { SleepChoice };

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

export type SleepState = SleepTimerView | null;

interface SleepStore {
  timer: SleepState;
}

export const useSleep = create<SleepStore>()(() => ({ timer: null }));

let followed: Backend | null = null;
let unlisten: (() => void) | null = null;

/** Load the core's timer and follow it (once per backend). */
export async function followSleep(b: Backend = backend): Promise<void> {
  if (followed === b) return;
  followed = b;
  unlisten = await b.listen("sleep://changed", (timer) => useSleep.setState({ timer }));
  const timer = await b.sleepGet().catch(() => null);
  useSleep.setState({ timer });
}

/** For tests: forget the subscription and the timer. */
export function resetSleep() {
  unlisten?.();
  unlisten = null;
  followed = null;
  useSleep.setState({ timer: null });
}

const report = (e: unknown) => {
  const msg = e && typeof e === "object" && "message" in e ? String((e as { message: unknown }).message) : "Couldn't set the sleep timer";
  toast.error(msg);
};

/** Start (or replace) the timer. */
export function startSleep(c: SleepChoice, b: Backend = backend): Promise<void> {
  return b.sleepSet(c).then((timer) => useSleep.setState({ timer }), report);
}

export function cancelSleep(b: Backend = backend): Promise<void> {
  useSleep.setState({ timer: null });
  return b.sleepCancel().catch(report);
}

/** The pill's line: what the timer is set to. */
export function sleepLabel(t: SleepState, now = Date.now()): string {
  if (!t) return "Sleep timer";
  if (t.kind === "track") return "End of song";
  if (t.kind === "album") return "End of album";
  const left = Math.max(0, Math.ceil((t.endsAtMs - now) / 60_000));
  return left >= 60 ? `${Math.floor(left / 60)} h ${left % 60 ? `${left % 60} min` : ""}`.trim() : `${left} min`;
}

/** The desktop button's tooltip: what happens and when. */
export function sleepTip(t: SleepState, now = Date.now()): string {
  if (!t) return "Sleep timer";
  if (t.kind === "track") return "Stops after this song";
  if (t.kind === "album") return "Stops after this album";
  return `Stops in ${sleepLabel(t, now)}`;
}

/** Whether the running timer is this choice. */
export function sameChoice(t: SleepState, c: SleepChoice): boolean {
  if (!t || t.kind !== c.kind) return false;
  return t.kind !== "minutes" || (c.kind === "minutes" && t.minutes === c.minutes);
}
