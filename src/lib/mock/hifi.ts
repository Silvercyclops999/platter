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

export function correctionFor(s: Settings, deviceId: string | undefined): DeviceCorrection | undefined {
  return deviceId ? s.headphoneCorrection.find((c) => c.deviceId === deviceId) : undefined;
}

export function setCorrection(s: Settings, device: OutputDevice, entry: HeadphoneEntry | null) {
  s.headphoneCorrection = s.headphoneCorrection.filter((c) => c.deviceId !== device.id);
  if (entry) s.headphoneCorrection = [...s.headphoneCorrection, { ...entry, deviceId: device.id, deviceName: device.name, ...profile(entry) }];
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
