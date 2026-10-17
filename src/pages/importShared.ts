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

/** "2020 to 2026", or "in 2024" for one year. */
function years(first: number, last: number): string {
  const a = new Date(first).getFullYear();
  const b = new Date(last).getFullYear();
  return a === b ? `in ${a}` : `${a} to ${b}`;
}

/** "153,497 plays imported, 2020 to 2026. 509 of 14,572 songs are in your library." Null without history. */
export function historyLine(r: ImportReport): string | null {
  const p = historyParts(r);
  return p && `${p.head}. ${p.songs}.`;
}

/** The two halves of the history line: "153,497 plays imported, 2020 to 2026" and "509 of 14,572 songs are in your library". */
export function historyParts(r: ImportReport): { head: string; songs: string } | null {
  const h = r.history;
  if (!h?.plays) return null;
  const when = h.firstPlayed != null && h.lastPlayed != null ? `, ${years(h.firstPlayed, h.lastPlayed)}` : "";
  const songs = r.songs.total ? "played songs" : h.songs === 1 ? "song" : "songs";
  const are = h.songs === 1 && !r.songs.total ? "is" : "are";
  return { head: `${plural(h.plays, "play")} imported${when}`, songs: `${n(h.songsMatched)} of ${n(h.songs)} ${songs} ${are} in your library` };
}

/** Whether an import brought in nothing to show. */
export function reportEmpty(r: ImportReport): boolean {
  return !r.songs.total && !r.history?.plays && !r.albums.total && !r.playlists && !r.liked.total;
}

/**
 * The one-line summary of an import: songs found from playlists and likes,
 * the listening history, or both. Says so plainly when nothing was usable.
 */
export function reportLine(r: ImportReport): string {
  const history = historyLine(r);
  if (r.songs.total && history) return `${songsLine(r)}. ${history}`;
  if (history) return history;
  if (r.songs.total) return songsLine(r);
  if (r.albums.total) return `${n(r.albums.matched)} of ${plural(r.albums.total, "saved album")} in your library`;
  return "Nothing in that file could be used: no songs, playlists or plays.";
}

/** How much of the import is in the library, 0 to 100 (songs, else played songs). */
export function reportPercent(r: ImportReport): number {
  if (r.songs.total) return Math.round((r.songs.matched / r.songs.total) * 100);
  if (r.history?.songs) return Math.round((r.history.songsMatched / r.history.songs) * 100);
  return 0;
}

/** The counts after the summary: liked, playlists, albums. Plays are in the summary. */
export function countParts(r: ImportReport): string[] {
  const out: string[] = [];
  if (r.liked.total) out.push(`${n(r.liked.matched)} of ${n(r.liked.total)} liked`);
  if (r.playlists) out.push(plural(r.playlists, "playlist"));
  if (r.albums.total && r.songs.total) out.push(`${n(r.albums.matched)} of ${plural(r.albums.total, "saved album")} in your library`);
  return out;
}

/** Open the Missing page on "Played a lot". */
export function showPlayedALot() {
  nav.go({ name: "import", view: "played" });
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
  missingFocus = null;
}

/** How the Missing list knows a song: folded artist and title. */
export function missingKey(artist: string, title: string): string {
  const f = (s: string) => s.normalize("NFKC").trim().replace(/\s+/g, " ").toLowerCase();
  return `${f(artist)}\u001f${f(title)}`;
}

let missingFocus: string | null = null;

/** Open the Missing list scrolled to one song. */
export function showInMissing(artist: string, title: string) {
  missingFocus = missingKey(artist, title);
  nav.go({ name: "import", view: "missing" });
}

/** The song the Missing list should bring into view. */
export function missingFocusKey(): string | null {
  return missingFocus;
}

/** Once the Missing list has shown that song. */
export function clearMissingFocus() {
  missingFocus = null;
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
    const historyOnly = !!r.history?.plays && !r.songs.total;
    useToasts.getState().push({
      kind: "info",
      message: `${importSourceName(r.source)}: ${reportLine(r)}`,
      action: historyOnly
        ? { label: "See Played a lot", run: showPlayedALot }
        : { label: "See report", run: () => nav.go({ name: "import" }) },
    });
    return r;
  } catch (e) {
    toast.error(`Couldn't import from ${importSourceName(source)}${errText(e) ? `: ${errText(e)}` : ""}`);
    return null;
  }
}
