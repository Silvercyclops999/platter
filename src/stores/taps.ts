/**
 * Songs tapped that the library didn't have. The core gets each one through
 * Lidarr and plays it when it's here (docs/IPC.md `lidarr_tap`); this keeps
 * the latest one for Now playing's "Getting it" line, from `lidarr://tap`.
 */
import { create } from "zustand";
import { backend } from "../lib/backend";
import type { AppError, Tap, TapInput, TapOutcome, TapPhase } from "../lib/types";
import { toast } from "./toasts";

interface TapsState {
  /** The latest tap, while it's on its way or playing from the download. */
  current: Tap | null;
}

export const useTaps = create<TapsState>(() => ({ current: null }));

/** Still coming: the player shows it in place of the seek bar. */
export function getting(t: Tap | null | undefined): t is Tap {
  return !!t && t.wanted && (t.phase === "searching" || t.phase === "downloading" || t.phase === "waiting" || t.phase === "importing" || t.phase === "failed");
}

/** The line in place of the seek bar. */
export function gettingLabel(phase: TapPhase, progress: number | null): string {
  switch (phase) {
    case "searching":
      return "Searching";
    case "downloading":
      return progress != null ? `Downloading ${progress}%` : "Downloading";
    case "waiting":
      return "Not found yet";
    case "importing":
      return "Ready in a moment";
    case "playing":
    case "ready":
      return "Ready";
    case "failed":
      return "Didn't come through";
  }
}

/** How far the line is filled, 0 to 1. */
export function gettingFill(phase: TapPhase, progress: number | null): number {
  switch (phase) {
    case "searching":
      return 0.04;
    case "downloading":
      return 0.08 + 0.84 * ((progress ?? 0) / 100);
    case "importing":
      return 0.96;
    case "playing":
    case "ready":
      return 1;
    default:
      return 0;
  }
}

function show(t: Tap) {
  const cur = useTaps.getState().current;
  if (t.wanted) useTaps.setState({ current: t });
  else if (cur?.id === t.id) useTaps.setState({ current: { ...t } });
}

let listening: Promise<unknown> | null = null;

/** Follow the core's taps (once). */
export function followTaps() {
  if (listening) return;
  listening = backend.listen("lidarr://tap", (t) => show(t));
}

/** Clear the line (the user dismissed a failed or waiting tap). */
export function dismissTap() {
  useTaps.setState({ current: null });
}

const errText = (e: unknown) => (e && typeof e === "object" && "message" in e ? String((e as AppError).message) : "");

/**
 * Tap a song the library doesn't have. Resolves with the outcome; `needs`
 * means the caller shows the request dialog (the very first request, or a
 * check said no).
 */
export async function tapToPlay(input: TapInput): Promise<TapOutcome | null> {
  followTaps();
  try {
    const out = await backend.lidarrTap(input);
    if (out.tap) show(out.tap);
    return out;
  } catch (e) {
    toast.error(errText(e) || `Couldn't get ${input.title}. Try again in a moment`);
    return null;
  }
}
