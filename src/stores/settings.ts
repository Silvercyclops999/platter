/**
 * App settings mirrored from the core (`settings_get` / `settings_set`).
 * Changes apply optimistically and roll back with a toast if the core says no.
 */
import { create } from "zustand";
import { backend, type Backend } from "../lib/backend";
import type { Settings } from "../lib/types";
import { toast } from "./toasts";

interface SettingsStore {
  settings: Settings | null;
  setLocal: (s: Settings) => void;
}

export const useSettings = create<SettingsStore>()((set) => ({
  settings: null,
  setLocal: (settings) => set({ settings }),
}));

export function loadSettings(b: Backend = backend): Promise<void> {
  return b.settingsGet().then(
    (s) => useSettings.getState().setLocal(s),
    () => {},
  );
}

/** Apply a change now and persist it. */
export async function updateSettings(patch: Partial<Settings>, b: Backend = backend): Promise<void> {
  const before = useSettings.getState().settings;
  if (before) useSettings.getState().setLocal({ ...before, ...patch });
  try {
    await b.settingsSet(patch);
  } catch (e) {
    if (before) useSettings.getState().setLocal(before);
    const msg = e && typeof e === "object" && "message" in e ? String((e as { message: unknown }).message) : "";
    toast.error(msg ? `Couldn't save that setting: ${msg}` : "Couldn't save that setting");
  }
}
