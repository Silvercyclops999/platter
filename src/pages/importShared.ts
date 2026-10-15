/** What the library import screens share: loading, running an import, and report wording. */
import { useCallback, useEffect, useState } from "react";
import { backend } from "../lib/backend";
import type { ImportOverview, ImportReport, ImportSkipped, ImportSourceId } from "../lib/importTypes";
import { importSourceName } from "../lib/importStrings";
import type { AppError } from "../lib/types";
import { nav } from "../stores/router";
import { toast, useToasts } from "../stores/toasts";

export const errText = (e: unknown) => (e && typeof e === "object" && "message" in e ? String((e as AppError).message) : "");

const n = (v: number) => v.toLocaleString();
const plural = (v: number, one: string, many = `${one}s`) => `${n(v)} ${v === 1 ? one : many}`;

/** "812 of 900 songs found". */
export function songsLine(r: ImportReport): string {
  return `${n(r.songs.matched)} of ${plural(r.songs.total, "song")} found`;
}

/** The counts after the songs line: liked, playlists, albums, plays. */
export function countParts(r: ImportReport): string[] {
  const out: string[] = [];
  if (r.liked.total) out.push(`${n(r.liked.matched)} of ${n(r.liked.total)} liked`);
  if (r.playlists) out.push(plural(r.playlists, "playlist"));
  if (r.albums.total) out.push(`${n(r.albums.matched)} of ${plural(r.albums.total, "saved album")} in your library`);
  if (r.plays.total) out.push(plural(r.plays.total, "play"));
  return out;
}

/** What was left out, in words, or null. */
export function skippedLine(s: ImportSkipped): string | null {
  const parts: string[] = [];
  if (s.privatePlays) parts.push(plural(s.privatePlays, "private play"));
  if (s.podcasts) parts.push(plural(s.podcasts, "podcast episode"));
  if (s.noMetadata) parts.push(plural(s.noMetadata, "play with no song name", "plays with no song name"));
  if (s.shortPlays) parts.push(plural(s.shortPlays, "play under 30 seconds", "plays under 30 seconds"));
  if (s.incompleteRows) parts.push(plural(s.incompleteRows, "row without an artist or title", "rows without an artist or title"));
  if (s.unknownFiles) parts.push(plural(s.unknownFiles, "other file", "other files"));
  return parts.length ? `Left out: ${parts.join(", ")}.` : null;
}

let listeners = new Set<() => void>();

/** Tell open import screens to load again. */
export function importChanged() {
  for (const l of listeners) l();
}

/** The import overview, kept fresh after imports and reviews. */
export function useImportOverview(): [ImportOverview | null, string | null, () => void] {
  const [data, setData] = useState<ImportOverview | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const load = useCallback(() => {
    backend.importOverview().then(
      (o) => {
        setData(o);
        setErr(null);
      },
      (e) => setErr(errText(e) || "Couldn't load your imports"),
    );
  }, []);
  useEffect(() => {
    load();
    listeners.add(load);
    return () => {
      listeners.delete(load);
    };
  }, [load]);
  return [data, err, load];
}

/** Reset between tests. */
export function resetImportListeners() {
  listeners = new Set();
}

/** Pick a file or folder and import it. Resolves to the report, or null when cancelled or failed. */
export async function importFrom(source: ImportSourceId, folder: boolean): Promise<ImportReport | null> {
  let path: string | null;
  try {
    path = await backend.importChoose(folder);
  } catch (e) {
    toast.error(`Couldn't open the picker${errText(e) ? `: ${errText(e)}` : ""}`);
    return null;
  }
  if (!path) return null;
  try {
    const r = await backend.importRun(source, path);
    importChanged();
    useToasts.getState().push({
      kind: "info",
      message: `${importSourceName(r.source)}: ${songsLine(r)}`,
      action: { label: "See report", run: () => nav.go({ name: "import" }) },
    });
    return r;
  } catch (e) {
    toast.error(`Couldn't import from ${importSourceName(source)}${errText(e) ? `: ${errText(e)}` : ""}`);
    return null;
  }
}
