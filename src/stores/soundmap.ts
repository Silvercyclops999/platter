/**
 * Whether the sound map has an entry in Explore on Home: its setting is on and
 * songs are analysed (or being analysed, with Automix on).
 */
import { useEffect } from "react";
import { create } from "zustand";
import { backend } from "../lib/backend";
import type { SoundMapInfo } from "../lib/types";
import { useSettings } from "./settings";

interface SoundMapState {
  info: SoundMapInfo | null;
  loading: boolean;
}

export const useSoundMap = create<SoundMapState>(() => ({ info: null, loading: false }));

export async function loadSoundMapInfo(): Promise<SoundMapInfo | null> {
  useSoundMap.setState({ loading: true });
  try {
    const info = await backend.soundmapInfo();
    useSoundMap.setState({ info, loading: false });
    return info;
  } catch {
    useSoundMap.setState({ loading: false });
    return null;
  }
}

/** The setting alone (the page itself says when it's off). */
export const useSoundMapOn = () => useSettings((s) => s.settings?.showSoundMap !== false);

/** Show the sound map's entries. */
export function useSoundMapEntry(): boolean {
  const on = useSoundMapOn();
  const automix = useSettings((s) => s.settings?.automix === true);
  const analysed = useSoundMap((s) => s.info?.analysed ?? 0);
  const known = useSoundMap((s) => s.info !== null || s.loading);
  useEffect(() => {
    if (on && !automix && !known) void loadSoundMapInfo();
  }, [on, automix, known]);
  return on && (automix || analysed > 0);
}
