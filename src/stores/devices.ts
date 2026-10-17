/**
 * Sync to a device as the UI sees it: the devices remembered on this
 * computer (and where each is plugged in), new removable drives, and the
 * sync running now, kept current from `devices://changed` and
 * `devices://progress`. Drives come and go without an event, so the
 * Devices page looks again every few seconds while it's open.
 */
import { create } from "zustand";
import { backend, type Backend } from "../lib/backend";
import type { AppError, Device, DeviceFormat, DeviceItemKind, DevicePatch, DeviceSyncStatus, DevicesView } from "../lib/types";
import { toast } from "./toasts";

interface DevicesState {
  view: DevicesView | null;
  error: string | null;
}

export const useDevices = create<DevicesState>()(() => ({ view: null, error: null }));

export const FORMAT_LABELS: Record<DeviceFormat, string> = {
  original: "Original",
  flac: "FLAC",
  "mp3-320": "MP3 320 kbps",
  "mp3-v0": "MP3 V0",
  opus: "Opus",
};
export const FORMAT_ORDER: DeviceFormat[] = ["original", "flac", "mp3-320", "mp3-v0", "opus"];

/** What's being sent to a device. */
export interface DeviceTarget {
  kind: DeviceItemKind;
  /** Empty for Liked Songs. */
  id: string;
  title: string;
}

const message = (e: unknown) => (e as AppError)?.message || "Something went wrong. Try again.";

let started: Backend | null = null;
let unsubs: Array<() => void> = [];

export async function loadDevices(b: Backend = backend): Promise<void> {
  try {
    const view = await b.devicesList();
    useDevices.setState({ view, error: null });
  } catch (e) {
    useDevices.setState({ error: message(e) });
  }
}

function takeProgress(s: DeviceSyncStatus) {
  const before = useDevices.getState().view;
  if (!before) return;
  const was = before.sync;
  useDevices.setState({ view: { ...before, sync: s } });
  if (was?.phase !== s.phase) {
    if (s.phase === "done") toast.info(`${s.deviceName} is synced. Safe to unplug`);
    if (s.phase === "failed" && s.error) toast.error(s.error.message);
  }
}

/** Load the devices and follow their events (once). */
export async function startDevices(b: Backend = backend): Promise<void> {
  if (started === b) return;
  started = b;
  unsubs.push(await b.listen("devices://changed", () => void loadDevices(b)));
  unsubs.push(await b.listen("devices://progress", takeProgress));
  await loadDevices(b);
}

/** For tests: forget the subscription and state. */
export function resetDevices() {
  unsubs.forEach((u) => u());
  unsubs = [];
  started = null;
  useDevices.setState({ view: null, error: null });
}

async function run<T>(what: string, f: () => Promise<T>): Promise<T | null> {
  try {
    const out = await f();
    await loadDevices();
    return out;
  } catch (e) {
    toast.error(`Couldn't ${what}: ${message(e)}`);
    return null;
  }
}

export const devices = {
  add: (root: string, name: string, folder: string): Promise<Device | null> => run("add the device", () => backend.devicesAdd(root, name, folder)),
  update: (id: string, patch: DevicePatch): Promise<Device | null> => run("change the device", () => backend.devicesUpdate(id, patch)),
  forget: (id: string) => run("forget the device", () => backend.devicesForget(id)),
  select: (id: string, t: DeviceTarget, on: boolean) => run(on ? "add it" : "take it off", () => backend.devicesSelect(id, t.kind, t.id, t.title, on)),
  sync: (id: string) => run("start the sync", () => backend.devicesSync(id)),
  pause: (on: boolean) => run(on ? "pause" : "resume", () => backend.devicesPause(on)),
  cancel: () => run("cancel", () => backend.devicesCancel()),
};

/** A sync that hasn't finished. */
export function running(s: DeviceSyncStatus | null | undefined): boolean {
  return !!s && (s.phase === "planning" || s.phase === "copying" || s.phase === "waiting" || s.phase === "paused");
}

/** Puts something on a device and syncs it now when it's plugged in. */
export async function sendToDevice(deviceId: string, t: DeviceTarget): Promise<void> {
  const d = await devices.select(deviceId, t, true);
  if (!d) return;
  const view = useDevices.getState().view;
  const plugged = view?.devices.find((x) => x.id === deviceId)?.root;
  if (plugged && !running(view?.sync)) {
    if (await devices.sync(deviceId)) toast.info(`Sending ${t.title} to ${d.name}`);
  } else if (plugged) {
    toast.info(`Added ${t.title} to ${d.name}. It goes on with the next sync`);
  } else {
    toast.info(`Added ${t.title} to ${d.name}. Plug it in and sync to copy it`);
  }
}

/** "3 of 12 songs", "Waiting while music plays". */
export function progressLine(s: DeviceSyncStatus): string {
  switch (s.phase) {
    case "planning":
      return "Getting ready…";
    case "waiting":
      return "Waiting until playback and your computer are less busy";
    case "paused":
      return `Paused, ${s.done} of ${s.total} ${s.total === 1 ? "song" : "songs"}`;
    case "copying":
      return `Copying ${s.done} of ${s.total} ${s.total === 1 ? "song" : "songs"}`;
    case "done":
      return s.total === 0 ? "Already up to date. Safe to unplug." : `Done: ${s.done} ${s.done === 1 ? "song" : "songs"} copied. Safe to unplug.`;
    case "cancelled":
      return "Cancelled. Safe to unplug.";
    case "failed":
      return s.error?.message ?? "The sync stopped.";
  }
}
