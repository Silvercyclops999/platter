/**
 * App settings mirrored from the core (`settings_get` / `settings_set`).
 * Changes apply optimistically and roll back with a toast if the core says no.
 */
import { create } from "zustand";
import { backend, type Backend } from "../lib/backend";
import type { Settings } from "../lib/types";
import { failText } from "../lib/errorText";
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
    if (before) rollBack(patch, before);
    toast.error(failText("Couldn't save that setting", e));
  }
}

/**
 * Undo a failed change: only the keys it set, and only those nothing has
 * changed since, so another setting changed meanwhile stays.
 */
function rollBack(patch: Partial<Settings>, before: Settings) {
  const now = useSettings.getState().settings;
  if (!now) return;
  const keys = (Object.keys(patch) as (keyof Settings)[]).filter((k) => now[k] === patch[k]);
  if (!keys.length) return;
  const back: Partial<Settings> = {};
  for (const k of keys) (back as Record<string, unknown>)[k] = before[k];
  useSettings.getState().setLocal({ ...now, ...back });
}
