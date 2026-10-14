/**
 * The core's audio settings rules for the browser mock: ranges, presets,
 * profiles and profiles bound to output devices (src-tauri/src/audio/eqprofile.rs).
 */
import { EQ_PRESETS, type OutputDevice, type Settings } from "../types";

const PROFILE = "profile:";
const NAME_MAX = 40;

const invalid = (message: string) => ({ kind: "invalid", message });
const notFound = (message: string) => ({ kind: "notFound", message });

const clampBand = (v: number) => (Number.isFinite(v) ? Math.min(12, Math.max(-12, Math.round(v * 10) / 10)) : 0);
const same = (a: number[], b: number[]) => a.length === b.length && a.every((v, i) => v === b[i]);

function findProfile(s: Settings, name: string) {
  return s.eqProfiles.find((p) => p.name.toLowerCase() === name.toLowerCase());
}

export function bandsOf(s: Settings, id: string): number[] | null {
  if (id.startsWith(PROFILE)) return findProfile(s, id.slice(PROFILE.length))?.bands ?? null;
  return EQ_PRESETS.find((p) => p.id === id)?.bands ?? null;
}

function select(s: Settings, id: string) {
  if (id === "custom") {
    s.eqPreset = "custom";
    return;
  }
  const bands = bandsOf(s, id);
  if (!bands) throw notFound(`EQ preset ${id}`);
  s.eqPreset = id;
  s.eqBands = [...bands];
}

/** Applies the audio part of a settings patch (throws an AppError). */
export function applyAudioPatch(s: Settings, patch: Partial<Settings>) {
  if (patch.preampDb !== undefined && !Number.isFinite(patch.preampDb)) throw invalid("preamp must be a number");
  if (typeof patch.eqPreset === "string") select(s, patch.eqPreset);
  if (Array.isArray(patch.eqBands)) {
    s.eqBands = patch.eqBands.slice(0, 10).map(clampBand);
    const cur = bandsOf(s, s.eqPreset);
    if (!cur || !same(cur, s.eqBands)) s.eqPreset = "custom";
  }
  s.crossfadeSeconds = Math.min(12, Math.max(1, Math.round(s.crossfadeSeconds)));
  s.preampDb = Math.min(12, Math.max(-12, s.preampDb));
}

function checkName(name: string): string {
  const n = name.trim();
  if (!n) throw invalid("Give the profile a name.");
  if (n.length > NAME_MAX) throw invalid(`Profile names can be up to ${NAME_MAX} characters.`);
  if (n.toLowerCase() === "custom" || EQ_PRESETS.some((p) => p.name.toLowerCase() === n.toLowerCase()))
    throw invalid(`${n} is a built-in preset. Pick another name.`);
  return n;
}

export function saveAs(s: Settings, name: string) {
  const n = checkName(name);
  const existing = findProfile(s, n);
  if (existing) {
    existing.name = n;
    existing.bands = [...s.eqBands];
  } else s.eqProfiles = [...s.eqProfiles, { name: n, bands: [...s.eqBands] }];
  s.eqPreset = PROFILE + n;
}

export function rename(s: Settings, from: string, to: string) {
  const n = checkName(to);
  const old = findProfile(s, from);
  if (!old) throw notFound(`EQ profile ${from}`);
  if (old.name.toLowerCase() !== n.toLowerCase() && findProfile(s, n)) throw invalid(`There's already a profile called ${n}.`);
  const [oldId, newId] = [PROFILE + old.name, PROFILE + n];
  s.eqProfiles = s.eqProfiles.map((p) => (p === old ? { ...p, name: n } : p));
  s.eqOutputs = s.eqOutputs.map((o) => (o.preset === oldId ? { ...o, preset: newId } : o));
  if (s.eqPreset === oldId) s.eqPreset = newId;
  if (s.eqFallback?.preset === oldId) s.eqFallback = { ...s.eqFallback, preset: newId };
}

export function remove(s: Settings, name: string) {
  const old = findProfile(s, name);
  if (!old) throw notFound(`EQ profile ${name}`);
  const id = PROFILE + old.name;
  s.eqProfiles = s.eqProfiles.filter((p) => p !== old);
  s.eqOutputs = s.eqOutputs.filter((o) => o.preset !== id);
  if (s.eqPreset === id) s.eqPreset = "custom";
  if (s.eqFallback?.preset === id) s.eqFallback = { ...s.eqFallback, preset: "custom" };
}

export function useForOutput(s: Settings, device: OutputDevice, on: boolean) {
  s.eqOutputs = s.eqOutputs.filter((o) => o.deviceId !== device.id);
  if (!on) return;
  if (s.eqPreset === "custom") throw invalid("Save this curve as a profile first.");
  s.eqOutputs = [...s.eqOutputs, { deviceId: device.id, deviceName: device.name, preset: s.eqPreset }];
}

function boundPreset(s: Settings, deviceId: string): string | null {
  const p = s.eqOutputs.find((o) => o.deviceId === deviceId)?.preset;
  return p && bandsOf(s, p) ? p : null;
}

/**
 * The device in use changed: its own preset or profile, or the choice for
 * devices without one (kept from the last such device). Returns whether
 * the EQ changed.
 */
export function outputChanged(s: Settings, device: OutputDevice | null): boolean {
  const id = device?.id ?? null;
  const last = s.eqLastOutput ?? null;
  if (last !== null && last === id) return false;
  if (last === null || !boundPreset(s, last)) s.eqFallback = { preset: s.eqPreset, bands: [...s.eqBands] };
  s.eqLastOutput = id;
  const [preset, bands] = [s.eqPreset, s.eqBands];
  const bound = id ? boundPreset(s, id) : null;
  if (bound) select(s, bound);
  else if (s.eqFallback) {
    s.eqPreset = s.eqFallback.preset;
    s.eqBands = [...s.eqFallback.bands];
  }
  return s.eqPreset !== preset || !same(s.eqBands, bands);
}

