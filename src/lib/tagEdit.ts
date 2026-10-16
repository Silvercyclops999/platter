/**
 * The tag editor's logic, apart from its dialogs: shared album values
 * ("(several)" when the files differ), turning what the user changed into
 * a save request, and the release type's two parts.
 */
import type { TagEditFile, TagField, TagFixChange, TagSaveFile, TagSaveRequest, TagValues } from "./types";

/** Album fields: the same on every file of an album. */
export const ALBUM_FIELDS: TagField[] = ["albumArtist", "album", "year", "genre", "releaseType", "edition", "group", "era", "quality", "explicit"];

/** More than this many files asks first (Settings). */
export const MANY_FILES = 50;

export const PRIMARY_TYPES = ["album", "ep", "single", "broadcast", "other"] as const;
export const SECONDARY_TYPES = ["compilation", "soundtrack", "live", "remix", "demo", "mixtape/street", "dj-mix", "spokenword", "interview", "audiobook"] as const;
export const QUALITIES = ["lossless", "cdq", "lq", "snippet"] as const;

export const FIELD_LABEL: Record<TagField, string> = {
  albumArtist: "Album artist",
  album: "Album",
  year: "Year",
  genre: "Genre",
  releaseType: "Release type",
  edition: "Edition",
  group: "Group",
  era: "Era",
  quality: "Quality",
  explicit: "Explicit",
  trackNumber: "Track",
  trackTotal: "Tracks",
  discNumber: "Disc",
  discTotal: "Discs",
  title: "Title",
  artist: "Artist",
  archiveVersion: "Release status",
  status: "Release status",
  picardStatus: "Release status",
  customStatus: "Release status",
  versionNote: "Version note",
  archiveDate: "Date",
  source: "Source",
  flags: "Flags",
  isrc: "ISRC",
};

/** A value every file shares, or null when they differ ("(several)"). Missing counts as "". */
export function shared(files: TagEditFile[], field: TagField): string | null {
  if (!files.length) return "";
  const first = files[0].values[field] ?? "";
  return files.every((f) => (f.values[field] ?? "") === first) ? first : null;
}

/** "album;demo" as its two parts. */
export function splitReleaseType(v: string): { primary: string; secondary: string } {
  const [p = "", ...rest] = v.split(";").map((x) => x.trim().toLowerCase());
  // A lone secondary type (the tag check flags it) shows as the second part.
  if (p && !(PRIMARY_TYPES as readonly string[]).includes(p)) return { primary: "", secondary: p };
  return { primary: p, secondary: rest.join(";") };
}

export function joinReleaseType(primary: string, secondary: string): string {
  if (!primary && !secondary) return "";
  if (!secondary) return primary;
  return `${primary || "album"};${secondary}`;
}

/**
 * The save request for what the user changed: album fields edited in the
 * album part go to every file, grid edits to their own file. Only values
 * that differ from a file's own are sent, so a field left at "(several)"
 * stays as each file has it. Files that couldn't be read are left out.
 */
export function buildRequest(
  label: string,
  files: TagEditFile[],
  albumEdits: Partial<Record<TagField, string>>,
  trackEdits: Record<string, Partial<Record<TagField, string>>>,
): TagSaveRequest {
  const out: TagSaveFile[] = [];
  for (const f of files) {
    if (!f.stamp) continue;
    const want: TagValues = { ...albumEdits, ...(trackEdits[f.trackId] ?? {}) };
    const edits: TagValues = {};
    for (const [k, v] of Object.entries(want) as [TagField, string][]) {
      if (v === undefined) continue;
      if ((f.values[k] ?? "") !== v.trim()) edits[k] = v.trim();
    }
    if (Object.keys(edits).length) out.push({ trackId: f.trackId, stamp: f.stamp, edits });
  }
  return { label, files: out };
}

/** A fix's picked changes as a save request. */
export function fixRequest(label: string, changes: TagFixChange[]): TagSaveRequest {
  const by = new Map<string, TagSaveFile>();
  for (const c of changes) {
    const f = by.get(c.trackId) ?? { trackId: c.trackId, stamp: c.stamp, edits: {} };
    f.edits[c.field] = c.after;
    by.set(c.trackId, f);
  }
  return { label, files: [...by.values()] };
}

/** Whether saving this many files asks first. */
export function asksFirst(files: number, setting: boolean | undefined): boolean {
  return setting !== false && files > MANY_FILES;
}
