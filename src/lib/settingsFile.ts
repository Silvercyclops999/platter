/**
 * The exported settings file, as the core writes it (settings.rs
 * `export_value` / `import_patch`). The mock backend and tests use this copy.
 */
import type { Settings, SettingsImportReport } from "./types";
import { DEFAULT_SETTINGS } from "./types";

/** Folders, devices and window places that belong to one computer. */
export const NOT_EXPORTED: readonly string[] = [
  "musicFolder",
  "musicFolderServer",
  "downloadFolder",
  "syncFolder",
  "miniplayerBounds",
  "outputDevice",
  "eqLastOutput",
  "lookStepDone",
  "libraries",
  "serverView",
];

export const EXPORT_KIND = "platter-settings";

export interface SettingsFile {
  kind: typeof EXPORT_KIND;
  version: 1;
  settings: Partial<Settings>;
}

export function exportSettings(s: Settings): SettingsFile {
  const settings: Record<string, unknown> = { ...s };
  for (const k of NOT_EXPORTED) delete settings[k];
  return { kind: EXPORT_KIND, version: 1, settings: settings as Partial<Settings> };
}

/** The patch a file makes. Unknown names and values of the wrong kind are skipped. */
export function importSettings(doc: unknown): { patch: Partial<Settings>; report: SettingsImportReport } {
  const d = doc as Partial<SettingsFile> | null;
  if (!d || d.kind !== EXPORT_KIND || !d.settings || typeof d.settings !== "object") {
    throw { kind: "invalid", message: "That file isn't a Platter settings file." };
  }
  const patch: Record<string, unknown> = {};
  const report: SettingsImportReport = { applied: 0, skipped: [] };
  const defaults = DEFAULT_SETTINGS as unknown as Record<string, unknown>;
  for (const [k, v] of Object.entries(d.settings)) {
    if (NOT_EXPORTED.includes(k)) continue;
    const want = defaults[k];
    const fits = k in defaults && (want === null || v === null || typeof want === typeof v);
    if (fits) {
      patch[k] = v;
      report.applied += 1;
    } else {
      report.skipped.push(k);
    }
  }
  return { patch: patch as Partial<Settings>, report };
}
