/** Album page logic: disc groups, the footer lines and the saved view. */
import type { Track } from "../lib/types";

export interface DiscGroup {
  /** null when the album has one disc: no separator row. */
  disc: number | null;
  tracks: Track[];
}

/**
 * Tracks split by disc, in the order given. One disc (or every track on the
 * same disc) gives a single group with no disc number, so no separator shows.
 */
export function discGroups(tracks: Track[]): DiscGroup[] {
  const discs = new Set(tracks.map((t) => t.discNumber || 1));
  if (discs.size <= 1) return [{ disc: null, tracks }];
  const out: DiscGroup[] = [];
  for (const t of tracks) {
    const d = t.discNumber || 1;
    const last = out[out.length - 1];
    if (last && last.disc === d) last.tracks.push(t);
    else out.push({ disc: d, tracks: [t] });
  }
  return out;
}

/** "2013-06-18" as a full date in the reader's language ("June 18, 2013"); null if it isn't a date. */
export function formatReleaseDate(released: string | null | undefined, locale?: string): string | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})/.exec(released ?? "");
  if (!m) return null;
  const [y, mo, d] = [Number(m[1]), Number(m[2]), Number(m[3])];
  if (mo < 1 || mo > 12 || d < 1 || d > 31) return null;
  return new Date(Date.UTC(y, mo - 1, d)).toLocaleDateString(locale, { year: "numeric", month: "long", day: "numeric", timeZone: "UTC" });
}

/** The label line under the release date: "© 2013 Label", or null without a label. */
export function labelLine(studio: string | null | undefined, year: number | null): string | null {
  const s = studio?.trim();
  if (!s) return null;
  return year ? `© ${year} ${s}` : `© ${s}`;
}

// ---------------------------------------------------------------- saved view (per device)

export type AlbumView = "list" | "compact";
export const ALBUM_VIEW_LABEL: Record<AlbumView, string> = { list: "List", compact: "Compact" };
const KEY = "platter.album-view.v1"; // gitleaks:allow (localStorage key name, not a secret)

export function loadAlbumView(): AlbumView {
  try {
    return localStorage.getItem(KEY) === "compact" ? "compact" : "list";
  } catch {
    return "list";
  }
}

export function saveAlbumView(v: AlbumView) {
  try {
    localStorage.setItem(KEY, v);
  } catch {
    /* storage unavailable: this session only */
  }
}
