/**
 * Playlists with Mix on and the transition picked for each gap, kept by
 * the core (`automix_mixed_get` / `automix_mix_set` / `automix_style_set`)
 * so they travel in the sync file. Changes show at once and go back if the
 * core says no; only the playlist changed goes back, and changes to one
 * playlist reach the core one at a time.
 */
import { create } from "zustand";
import { backend, type Backend } from "../lib/backend";
import type { MixStyle, MixedPlaylists } from "../lib/types";
import { toast } from "./toasts";

interface MixedState {
  mixed: MixedPlaylists;
  loaded: boolean;
}

export const useMixed = create<MixedState>()(() => ({ mixed: {}, loaded: false }));

export const gapKey = (fromId: string, toId: string) => `${fromId}>${toId}`;

/** Names for the transition styles, in menu order. */
export const MIX_STYLES: { style: MixStyle; label: string; hint: string }[] = [
  { style: "auto", label: "Auto", hint: "Platter picks a transition for these two songs" },
  { style: "none", label: "No transition", hint: "Straight on to the next song" },
  { style: "blend", label: "Blend", hint: "Both songs cross evenly" },
  { style: "lift", label: "Lift", hint: "The next song comes up first, then this one goes" },
  { style: "drop", label: "Drop", hint: "The next song waits, then takes over on the bar" },
  { style: "echo", label: "Echo out", hint: "This song echoes away as the next one comes in" },
];

export const styleLabel = (s: MixStyle) => MIX_STYLES.find((m) => m.style === s)?.label ?? "Auto";

export async function loadMixed(b: Backend = backend): Promise<void> {
  try {
    const mixed = await b.automixMixedGet();
    useMixed.setState({ mixed, loaded: true });
  } catch (e) {
    // Keep what's shown; nothing more to wait for.
    console.warn("Couldn't load mixed playlists", e);
    useMixed.setState({ loaded: true });
  }
}

type Entry = MixedPlaylists[string] | undefined;

/** Puts one playlist's entry back (or removes it), leaving the rest alone. */
function putEntry(id: string, entry: Entry) {
  useMixed.setState((s) => {
    const mixed = { ...s.mixed };
    if (entry) mixed[id] = entry;
    else delete mixed[id];
    return { mixed };
  });
}

/** Changes to each playlist waiting for the core, in order. */
const queues = new Map<string, Promise<unknown>>();
const waiting = new Map<string, number>();

/**
 * Sends one playlist's change after the ones before it. The core's answer
 * is shown, except for playlists that still have changes on the way (they
 * keep what's shown until theirs come back).
 */
function send(id: string, call: () => Promise<MixedPlaylists>): Promise<void> {
  waiting.set(id, (waiting.get(id) ?? 0) + 1);
  const run = (queues.get(id) ?? Promise.resolve())
    .catch(() => {})
    .then(call)
    .then((fromCore) => {
      useMixed.setState((s) => {
        const mixed = { ...fromCore };
        for (const [other, n] of waiting) {
          if (n <= (other === id ? 1 : 0)) continue;
          if (s.mixed[other]) mixed[other] = s.mixed[other];
          else delete mixed[other];
        }
        return { mixed };
      });
    })
    .finally(() => {
      const n = (waiting.get(id) ?? 1) - 1;
      if (n > 0) waiting.set(id, n);
      else waiting.delete(id);
      if (queues.get(id) === run) queues.delete(id);
    });
  queues.set(id, run);
  return run;
}

/** Mix on or off for a playlist. */
export async function setMix(id: string, on: boolean, b: Backend = backend): Promise<void> {
  const before: Entry = useMixed.getState().mixed[id];
  putEntry(id, on ? (before ?? { styles: {} }) : undefined);
  try {
    await send(id, () => b.automixMixSet(id, on));
  } catch {
    putEntry(id, before);
    toast.error(on ? "Couldn't turn on mixing" : "Couldn't turn off mixing");
  }
}

/** The transition between two songs of a mixed playlist. */
export async function setGapStyle(id: string, fromId: string, toId: string, style: MixStyle, b: Backend = backend): Promise<void> {
  const before: Entry = useMixed.getState().mixed[id];
  if (!before) return;
  const styles = { ...before.styles };
  if (style === "auto") delete styles[gapKey(fromId, toId)];
  else styles[gapKey(fromId, toId)] = style;
  putEntry(id, { ...before, styles });
  try {
    await send(id, () => b.automixStyleSet(id, fromId, toId, style));
  } catch {
    putEntry(id, before);
    toast.error("Couldn't change the transition");
  }
}

/** Mixed playlists changed on another device: read them again (once they've been read here). */
export function watchMixed(b: Backend = backend): Promise<() => void> {
  return b.listen("syncstore://changed", (c) => {
    if (c.remote && c.namespaces.includes("mixedPlaylists") && useMixed.getState().loaded) void loadMixed(b);
  });
}
