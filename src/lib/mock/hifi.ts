/**
 * Headphone correction and bit-perfect output for the browser mock: a few
 * invented headphones, made-up profiles, and the per-device rules of
 * src-tauri/src/audio/hifi.rs.
 */
import type { DeviceCorrection, Exclusive, HeadphoneEntry, OutputDevice, PeqFilter, Settings } from "../types";

/** Invented headphones (the real index ships with the core). */
export const MOCK_HEADPHONES: HeadphoneEntry[] = [
  { name: "Aurora Studio 600", source: "Lab One", form: "over-ear", path: "Lab One/over-ear/Aurora Studio 600" },
  { name: "Aurora Studio 600", source: "Bench Two on Rig A", form: "over-ear", path: "Bench Two/Rig A over-ear/Aurora Studio 600" },
  { name: "Aurora Studio 650", source: "Lab One", form: "over-ear", path: "Lab One/over-ear/Aurora Studio 650" },
  { name: "Kestrel Open Back", source: "Lab One", form: "over-ear", path: "Lab One/over-ear/Kestrel Open Back" },
  { name: "Kestrel Buds Pro", source: "Bench Two on Rig A", form: "in-ear", path: "Bench Two/Rig A in-ear/Kestrel Buds Pro" },
  { name: "Tern IEM 2", source: "Bench Two on Rig A", form: "in-ear", path: "Bench Two/Rig A in-ear/Tern IEM 2" },
  { name: "Tern Earbud", source: "Lab One", form: "earbud", path: "Lab One/earbud/Tern Earbud" },
];

const fold = (s: string) => s.normalize("NFKD").replace(/[̀-ͯ]/g, "").toLowerCase();

export function search(query: string): HeadphoneEntry[] {
  const q = fold(query.trim());
  const words = q.split(/\s+/).filter(Boolean);
  if (!words.length) return [];
  return MOCK_HEADPHONES.filter((h) => words.every((w) => fold(h.name).includes(w))).slice(0, 50);
}

/** A believable profile: a bass shelf, a few peaks, a treble shelf. */
export function profile(entry: HeadphoneEntry): { preampDb: number; filters: PeqFilter[] } {
  const seed = [...entry.path].reduce((n, c) => (n * 31 + c.charCodeAt(0)) % 997, 7);
  const g = (i: number) => Math.round((((seed * (i + 3)) % 90) / 10 - 4.5) * 10) / 10;
  const filters: PeqFilter[] = [
    { kind: "lowShelf", freq: 105, gainDb: Math.abs(g(0)) + 2, q: 0.7 },
    { kind: "peaking", freq: 180, gainDb: g(1), q: 0.6 },
    { kind: "peaking", freq: 1300, gainDb: g(2), q: 2.1 },
    { kind: "peaking", freq: 3200, gainDb: g(3), q: 3.7 },
    { kind: "peaking", freq: 6600, gainDb: g(4), q: 5.8 },
    { kind: "highShelf", freq: 10000, gainDb: g(5), q: 0.7 },
  ];
  const preampDb = -Math.max(0, ...filters.map((f) => f.gainDb));
  return { preampDb, filters };
}

/** The correction `deviceId` uses; one Platter picked counts only while Tune for my headphones is on. */
export function correctionFor(s: Settings, deviceId: string | undefined): DeviceCorrection | undefined {
  return deviceId ? s.headphoneCorrection.find((c) => c.deviceId === deviceId && (!c.auto || s.tuneHeadphones)) : undefined;
}

/** Set by hand: off also stops Platter picking one there again; a pick lets it. */
export function setCorrection(s: Settings, device: OutputDevice, entry: HeadphoneEntry | null) {
  s.headphoneDeclined = [...(s.headphoneDeclined ?? []).filter((id) => id !== device.id), ...(entry ? [] : [device.id])];
  bind(s, device, entry, false);
}

function bind(s: Settings, device: OutputDevice, entry: HeadphoneEntry | null, auto: boolean) {
  s.headphoneCorrection = s.headphoneCorrection.filter((c) => c.deviceId !== device.id);
  if (entry) s.headphoneCorrection = [...s.headphoneCorrection, { ...entry, deviceId: device.id, deviceName: device.name, ...profile(entry), auto }];
}

/** The output's name cut down to the model, as the core does ("Headphones (2- X Stereo)" is "X"). */
export function headphoneQuery(name: string): string {
  const noise = ["bluetooth", "stereo", "hands-free", "headphones", "(2)", "(3)"];
  return name
    .replace(/^[^\s']+['\u2019]s /, "")
    .split(/\s+/)
    .filter((w) => !noise.includes(w.toLowerCase()))
    .map((w) => w.replace(/^[()]+|[()]+$/g, ""))
    .filter((w) => w && !/^\d+-$/.test(w) && !noise.includes(w.toLowerCase()))
    .join(" ");
}

/**
 * The model a name is, exactly, maker in front or not ("Studio 600" is
 * "Aurora Studio 600"); not one it starts ("Kestrel"), and not one with a
 * later generation the name can't tell apart ("X Pro" and "X Pro 2").
 */
export function modelMatch(name: string): HeadphoneEntry | null {
  const q = fold(headphoneQuery(name));
  if (!q) return null;
  const isModel = (n: string) => n === q || (n.endsWith(` ${q}`) && !n.slice(0, -q.length - 1).includes(" "));
  const hits = MOCK_HEADPHONES.filter((h) => isModel(fold(h.name)));
  const first = hits[0];
  if (!first || hits.some((h) => fold(h.name) !== fold(first.name))) return null;
  const f = fold(first.name);
  if (MOCK_HEADPHONES.some((h) => fold(h.name).startsWith(f) && /^\s*\d/.test(fold(h.name).slice(f.length)))) return null;
  return first;
}

const TUNED_ROUTES = ["headphones", "bluetooth", "usb"];

/** Headphones, never set by hand or turned off there, with the switch on. */
export function mayTune(s: Settings, device: OutputDevice): boolean {
  if (!s.tuneHeadphones || device.id.startsWith("net:")) return false;
  if (s.bitPerfectOutputs.some((b) => b.deviceId === device.id)) return false;
  if (device.id.startsWith("route:") && !TUNED_ROUTES.includes(device.id.split(":")[1])) return false;
  return !s.headphoneCorrection.some((c) => c.deviceId === device.id) && !(s.headphoneDeclined ?? []).includes(device.id);
}

/** Tunes `device` when Platter may and knows its model; returns the headphone's name. */
export function autoTune(s: Settings, device: OutputDevice | null): string | null {
  // The sound moved on: the EQ follows the output it's on now.
  if (!device || !mayTune(s, device) || s.eqLastOutput !== device.id) return null;
  const entry = modelMatch(device.name);
  if (!entry) return null;
  bind(s, device, entry, true);
  return entry.name;
}

export function setBitPerfect(s: Settings, device: OutputDevice, on: boolean) {
  s.bitPerfectOutputs = s.bitPerfectOutputs.filter((b) => b.deviceId !== device.id);
  if (on) s.bitPerfectOutputs = [...s.bitPerfectOutputs, { deviceId: device.id, deviceName: device.name }];
}

/**
 * How the mock's output holds `device`. The receiver on HDMI can't go
 * exclusive at a hi-res rate, so it falls back, to show that state.
 */
export function exclusiveFor(s: Settings, device: OutputDevice | null, rate: number | null | undefined, bitDepth: number | null | undefined): Exclusive {
  if (!device || !s.bitPerfectOutputs.some((b) => b.deviceId === device.id)) return { state: "off" };
  const r = rate ?? 44100;
  if (device.id === "mock:hdmi" && r > 48000) return { state: "fallback", reason: `The device can't play ${r / 1000} kHz directly.` };
  return { state: "on", rate: r, bits: bitDepth && bitDepth > 16 ? 24 : 16 };
}
