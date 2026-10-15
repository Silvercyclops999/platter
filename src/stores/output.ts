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

/** One entry in the output list. `id` null is "System default". */
export interface OutputChoice {
  id: string | null;
  label: string;
  /** For "System default": the device it means right now. */
  defaultName?: string;
  selected: boolean;
  isDefault: boolean;
}

/**
 * The output list shown in the device panel and in Settings: "System
 * default (<device>)" first, then every output the core reports, in its
 * order, with the current one included. A chosen device that isn't plugged
 * in stays at the end as "Disconnected device" until another is picked.
 */
export function outputChoices(state: OutputState): OutputChoice[] {
  const def = state.devices.find((d) => d.isDefault) ?? (state.selected == null ? state.current : null);
  const list: OutputChoice[] = [
    {
      id: null,
      label: def ? `System default (${def.name})` : "System default",
      defaultName: def?.name,
      selected: state.selected == null,
      isDefault: false,
    },
  ];
  const seen = new Set<string>();
  for (const d of state.devices) {
    if (seen.has(d.id)) continue;
    seen.add(d.id);
    list.push({ id: d.id, label: d.name, selected: state.selected === d.id, isDefault: d.isDefault });
  }
  // The core puts the device in use in its list; this is a guard in case it doesn't.
  const cur = state.current;
  if (cur && !seen.has(cur.id)) {
    seen.add(cur.id);
    list.splice(1, 0, { id: cur.id, label: cur.name, selected: state.selected === cur.id, isDefault: cur.isDefault });
  }
  if (state.selected && !seen.has(state.selected)) {
    list.push({ id: state.selected, label: "Disconnected device", selected: true, isDefault: false });
  }
  return list;
}

/** Toast text when the chosen device goes away. */
export const outputLostMessage = (device: string) => `${device} disconnected. Switched to the system default.`;
