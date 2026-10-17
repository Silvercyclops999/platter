/**
 * Playlists with Mix on and the transition picked for each gap, kept by
 * the core (`automix_mixed_get` / `automix_mix_set` / `automix_style_set`)
 * so they travel in the sync file. Changes show at once and go back if the
 * core says no.
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
  } catch {
    /* keep what's shown */
  }
}

/** Mix on or off for a playlist. */
export async function setMix(id: string, on: boolean, b: Backend = backend): Promise<void> {
  const before = useMixed.getState().mixed;
  const next = { ...before };
  if (on) next[id] = before[id] ?? { styles: {} };
  else delete next[id];
  useMixed.setState({ mixed: next });
  try {
    useMixed.setState({ mixed: await b.automixMixSet(id, on) });
  } catch {
    useMixed.setState({ mixed: before });
    toast.error(on ? "Couldn't turn on mixing" : "Couldn't turn off mixing");
  }
}

/** The transition between two songs of a mixed playlist. */
export async function setGapStyle(id: string, fromId: string, toId: string, style: MixStyle, b: Backend = backend): Promise<void> {
  const before = useMixed.getState().mixed;
  const p = before[id];
  if (!p) return;
  const styles = { ...p.styles };
  if (style === "auto") delete styles[gapKey(fromId, toId)];
  else styles[gapKey(fromId, toId)] = style;
  useMixed.setState({ mixed: { ...before, [id]: { styles } } });
  try {
    useMixed.setState({ mixed: await b.automixStyleSet(id, fromId, toId, style) });
  } catch {
    useMixed.setState({ mixed: before });
    toast.error("Couldn't change the transition");
  }
}
