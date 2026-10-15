/**
 * Library doctor settings: which checks run (each its own switch), the
 * small cover size, and a way to the page with the current count.
 */
import { useId } from "react";
import { Switch } from "../components/Switch";
import type { SelectOption } from "../components/Select";
import type { DoctorChecks, Settings } from "../lib/types";
import { DEFAULT_SETTINGS } from "../lib/types";
import { useDoctorCount } from "../stores/doctor";
import { nav } from "../stores/router";
import { updateSettings } from "../stores/settings";
import "./doctor-settings.css";

/** Each check, in the page's order, with what it looks for. */
export const DOCTOR_CHECKS: { key: keyof DoctorChecks; label: string; hint: string }[] = [
  { key: "duplicates", label: "Duplicate files", hint: "The same recording more than once on one album." },
  { key: "covers", label: "Missing or small covers", hint: "Albums with no cover, or one smaller than the size below." },
  { key: "albumTags", label: "Album tags that differ", hint: "Album name, album artist or year that aren't the same on every song of an album." },
  { key: "splitAlbums", label: "Albums split in two", hint: "One album Plex shows as two, with its songs divided between them." },
  { key: "albumArtist", label: "Missing album artist", hint: "Songs with no album artist on albums that credit several artists." },
  { key: "lowQuality", label: "Low-quality copies", hint: "A song you also have in better quality on another album." },
  { key: "unmatched", label: "Albums Plex didn't match", hint: "Albums MusicBrainz knows but Plex couldn't match. Unreleased albums are never listed." },
];

export const COVER_PX_OPTIONS = [
  { value: "300", label: "300 pixels" },
  { value: "500", label: "500 pixels" },
  { value: "600", label: "600 pixels" },
  { value: "800", label: "800 pixels" },
  { value: "1000", label: "1000 pixels" },
  { value: "1400", label: "1400 pixels" },
] as const satisfies SelectOption<string>[];

/** "Open library doctor" with the count, under Run library doctor. */
export function DoctorNote({ on }: { on: boolean }) {
  const count = useDoctorCount();
  if (!on) return null;
  return (
    <button type="button" className="link" onClick={() => nav.go({ name: "doctor" })}>
      {count > 0 ? `Open library doctor (${count} to fix)` : "Open library doctor"}
    </button>
  );
}

/** One switch per check. */
export function DoctorChecksList({ settings }: { settings: Settings }) {
  const id = useId();
  const checks = settings.doctorChecks ?? DEFAULT_SETTINGS.doctorChecks;
  return (
    <ul className="doctor-checks" aria-label="Library doctor checks">
      {DOCTOR_CHECKS.map((c) => (
        <li key={c.key} className="doctor-checks__row">
          <span className="doctor-checks__text">
            <span id={`${id}-${c.key}`} className="doctor-checks__label">
              {c.label}
            </span>
            <span id={`${id}-${c.key}-d`} className="doctor-checks__hint">
              {c.hint}
            </span>
          </span>
          <Switch
            checked={checks[c.key] !== false}
            disabled={!settings.libraryDoctor}
            onChange={(v) => void updateSettings({ doctorChecks: { ...checks, [c.key]: v } })}
            labelledBy={`${id}-${c.key}`}
            describedBy={`${id}-${c.key}-d`}
          />
        </li>
      ))}
    </ul>
  );
}
