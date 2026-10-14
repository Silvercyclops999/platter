/**
 * Audio output devices, mirrored from the core (`output_devices`,
 * `output_select`, `output://changed`). Loaded when the picker or Settings
 * needs it; kept current by the core's events after that.
 */
import { create } from "zustand";
import { backend, type Backend } from "../lib/backend";
import type { OutputState } from "../lib/types";
import { useSettings } from "./settings";
import { toast } from "./toasts";

interface OutputStore {
  state: OutputState | null;
  /** A switch is in flight (to this id, or null for the system default). */
  switching: string | null | undefined;
  set: (s: OutputState) => void;
}

export const useOutput = create<OutputStore>()((set) => ({
  state: null,
  switching: undefined,
  set: (state) => set({ state }),
}));

export function loadOutput(b: Backend = backend): Promise<void> {
  return b.outputDevices().then(
    (s) => useOutput.getState().set(s),
    () => {},
  );
}

/** Play on `deviceId` (null: the system default). Position and play state carry over. */
export async function selectOutput(deviceId: string | null, b: Backend = backend): Promise<void> {
  if (useOutput.getState().state?.selected === deviceId) return;
  useOutput.setState({ switching: deviceId });
  try {
    const s = await b.outputSelect(deviceId);
    useOutput.getState().set(s);
    const settings = useSettings.getState().settings;
    if (settings) useSettings.getState().setLocal({ ...settings, outputDevice: s.selected });
  } catch (e) {
    const msg = e && typeof e === "object" && "message" in e ? String((e as { message: unknown }).message) : "";
    toast.error(msg ? `Couldn't switch the output: ${msg}` : "Couldn't switch the output");
    void loadOutput(b);
  } finally {
    useOutput.setState({ switching: undefined });
  }
}

/** Toast text when the chosen device goes away. */
export const outputLostMessage = (device: string) => `${device} disconnected. Switched to the system default.`;
