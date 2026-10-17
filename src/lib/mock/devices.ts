/**
 * Sync to a device in the browser mock (docs/IPC.md "Devices"): one
 * removable drive that can be plugged in and out, devices remembered in
 * memory, and a sync that moves through its phases on timers. Test hooks:
 * `plugged`, `fail` (the device fills up, is write-protected or is pulled
 * out partway), `stepMs`, and `reset()`.
 */
import type { Device, DeviceItemKind, DevicePatch, DeviceSyncStatus, DeviceView, DevicesView, DriveVolume } from "../types";
import type { Fixtures } from "./fixtures";

const GB = 1024 * 1024 * 1024;
const FORMATS = ["original", "flac", "mp3-320", "mp3-v0", "opus"];

export const MOCK_DRIVE: DriveVolume = {
  root: "E:\\",
  label: "WALKMAN",
  id: "4A1F-09C2",
  removable: true,
  free: 21.4 * GB,
  total: 29.7 * GB,
};

type Fail = "full" | "writeProtected" | "accessDenied" | "removed" | null;

const invalid = (message: string) => ({ kind: "invalid", message });

export class MockDevices {
  devices: Device[] = [];
  /** Test hooks. */
  plugged = true;
  fail: Fail = null;
  /** Shown with the next sync (say, a damaged manifest kept aside). */
  notice: string | null = null;
  stepMs = 40;
  status: DeviceSyncStatus | null = null;
  private seq = 1;
  private paused = false;
  private cancelled = false;

  constructor(
    private fx: Fixtures,
    private likedIds: () => string[],
    private later: (ms: number, fn: () => void) => void,
    private emit: (event: "devices://progress" | "devices://changed", payload: DeviceSyncStatus | null) => void,
  ) {}

  reset() {
    this.devices = [];
    this.plugged = true;
    this.fail = null;
    this.notice = null;
    this.stepMs = 40;
    this.status = null;
    this.paused = false;
    this.cancelled = false;
  }

  private changed() {
    this.later(0, () => this.emit("devices://changed", null));
  }

  private find(id: string): Device {
    const d = this.devices.find((x) => x.id === id);
    if (!d) throw { kind: "notFound", message: "That device isn't on this computer any more." };
    return d;
  }

  private rootOf(d: Device): DriveVolume | null {
    return this.plugged && d.volumeId === MOCK_DRIVE.id ? MOCK_DRIVE : null;
  }

  view(): DevicesView {
    const devices: DeviceView[] = this.devices.map((d) => {
      const v = this.rootOf(d);
      return { ...d, root: v?.root ?? null, free: v?.free ?? null, total: v?.total ?? null };
    });
    const used = devices.some((d) => d.root === MOCK_DRIVE.root);
    return { devices, drives: this.plugged && !used ? [MOCK_DRIVE] : [], sync: this.status };
  }

  add(root: string, name: string, folder: string): Device {
    if (!this.plugged || root !== MOCK_DRIVE.root) throw { kind: "notFound", message: "That drive isn't plugged in any more." };
    const n = name.trim().slice(0, 60);
    if (!n) throw invalid("Give the device a name.");
    const d: Device = {
      id: `dev${this.seq++}`,
      name: n,
      volumeId: MOCK_DRIVE.id,
      volumeLabel: MOCK_DRIVE.label,
      folder: cleanFolder(folder),
      format: null,
      budgetGb: null,
      selection: [],
      lastSync: null,
      lastRoot: MOCK_DRIVE.root,
    };
    this.devices.push(d);
    this.changed();
    return d;
  }

  update(id: string, p: DevicePatch): Device {
    const d = { ...this.find(id) };
    if (p.name !== undefined) {
      const n = p.name.trim().slice(0, 60);
      if (!n) throw invalid("Give the device a name.");
      d.name = n;
    }
    if (p.folder !== undefined) d.folder = cleanFolder(p.folder);
    if (p.format !== undefined) {
      if (p.format !== "default" && !FORMATS.includes(p.format)) throw invalid(`Unknown format ${p.format}.`);
      d.format = p.format === "default" ? null : p.format;
    }
    if (p.budgetGb !== undefined) {
      if (!Number.isFinite(p.budgetGb) || p.budgetGb < 0) throw invalid("Size limits go from 0 (no limit) up.");
      d.budgetGb = p.budgetGb > 0 ? p.budgetGb : null;
    }
    this.devices = this.devices.map((x) => (x.id === id ? d : x));
    this.changed();
    return d;
  }

  select(id: string, kind: DeviceItemKind, itemId: string, title: string, on: boolean): Device {
    const d = { ...this.find(id), selection: [...this.find(id).selection] };
    const key = kind === "liked" ? "" : itemId;
    const at = d.selection.findIndex((s) => s.kind === kind && (kind === "liked" || s.id === key));
    if (on && at < 0) d.selection.push({ kind, id: key, title: kind === "liked" ? "Liked Songs" : title });
    if (!on && at >= 0) d.selection.splice(at, 1);
    this.devices = this.devices.map((x) => (x.id === id ? d : x));
    this.changed();
    return d;
  }

  forget(id: string) {
    if (this.status && this.running() && this.status.deviceId === id) throw invalid("Cancel the sync to this device first.");
    this.find(id);
    this.devices = this.devices.filter((d) => d.id !== id);
    this.changed();
  }

  private running() {
    return !!this.status && ["planning", "copying", "waiting", "paused"].includes(this.status.phase);
  }

  /** Songs an item holds, from the fixture library. */
  songs(kind: DeviceItemKind, id: string): string[] {
    switch (kind) {
      case "album":
        return (this.fx.tracksByAlbum.get(id) ?? []).map((t) => t.id);
      case "playlist":
        return this.fx.playlistTracks.get(id) ?? [];
      case "artist":
        return this.fx.tracks.filter((t) => t.artistId === id).map((t) => t.id);
      case "liked":
        return this.likedIds();
    }
  }

  sync(id: string): DeviceSyncStatus {
    const d = this.find(id);
    if (this.running()) throw invalid("A sync is already running. Wait for it, or cancel it.");
    this.paused = false;
    this.cancelled = false;
    const seen = new Set<string>();
    for (const s of d.selection) for (const t of this.songs(s.kind, s.id)) seen.add(t);
    const total = seen.size;
    const st: DeviceSyncStatus = {
      deviceId: d.id,
      deviceName: d.name,
      phase: "planning",
      total: 0,
      done: 0,
      kept: 0,
      removed: 0,
      current: null,
      bytes: 0,
      failed: [],
      leftOut: [],
      leftOutSongs: 0,
      error: null,
      notice: null,
      safeToUnplug: false,
    };
    this.status = st;
    const put = (patch: Partial<DeviceSyncStatus>) => {
      this.status = { ...this.status!, ...patch };
      this.emit("devices://progress", this.status);
    };
    if (!this.rootOf(d)) {
      this.later(0, () => put({ phase: "failed", safeToUnplug: true, error: { kind: "notConnected", message: `Plug in ${d.name} to sync it.` } }));
      return st;
    }
    const ids = [...seen];
    const step = (i: number) => {
      if (this.cancelled) return put({ phase: "cancelled", current: null, safeToUnplug: true });
      if (this.paused) return this.later(this.stepMs, () => step(i));
      if (this.fail === "writeProtected") {
        return put({ phase: "failed", error: { kind: "writeProtected", message: "The device is write-protected. Slide its lock switch off, or make sure it isn't read-only, then sync again." }, safeToUnplug: true });
      }
      if (this.fail === "accessDenied") {
        return put({
          phase: "failed",
          error: {
            kind: "accessDenied",
            message: "Platter isn't allowed to write to the music folder on the device. Check that the folder isn't read-only and that you can change it, then sync again.",
          },
          safeToUnplug: true,
        });
      }
      if (this.notice && i === 0) put({ notice: this.notice });
      if (this.fail === "full" && i >= 2) {
        return put({ phase: "failed", current: null, error: { kind: "full", message: "The device is full. Free some space on it, set a smaller size limit, or choose less, then sync again." }, safeToUnplug: true });
      }
      if (this.fail === "removed" && i >= 1) {
        return put({ phase: "failed", current: null, error: { kind: "removed", message: "The device was removed before the sync finished. Plug it back in and sync again; the songs already copied stay." }, safeToUnplug: false });
      }
      if (i >= ids.length) {
        this.devices = this.devices.map((x) => (x.id === d.id ? { ...x, lastSync: Math.floor(Date.now() / 1000) } : x));
        this.changed();
        return put({ phase: "done", done: ids.length, current: null, safeToUnplug: true });
      }
      const t = this.fx.tracks.find((x) => x.id === ids[i]);
      put({ phase: "copying", done: i, current: t?.title ?? null, bytes: i * 9_400_000 });
      this.later(this.stepMs, () => step(i + 1));
    };
    this.later(0, () => {
      put({ total });
      step(0);
    });
    return st;
  }

  pause(on: boolean) {
    if (!this.running()) return;
    this.paused = on;
    this.status = { ...this.status!, phase: on ? "paused" : "copying" };
    this.emit("devices://progress", this.status);
  }

  cancel() {
    if (this.running()) this.cancelled = true;
    if (this.paused) {
      this.paused = false;
    }
  }
}

function cleanFolder(s: string): string {
  const parts = s.split(/[\\/]/).map((p) => p.trim()).filter((p) => p && p !== ".");
  if (parts.includes("..")) throw invalid("The music folder has to be on the device.");
  return parts.join("/");
}
