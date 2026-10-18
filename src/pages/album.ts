/** Album page logic: disc groups, the footer lines and the saved view. */
import type { AlbumCreditRow, AlbumCredits, AlbumSide, Track } from "../lib/types";

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

export interface TrackGroup {
  /** A disc heading before the group (albums of more than one disc). */
  disc: number | null;
  /** A side heading before the group, the first time a side shows. */
  side: AlbumSide | null;
  tracks: Track[];
}

/**
 * Tracks split by disc and, when the album has sides, by side. A side's
 * heading shows once, where it starts; songs no side lists go on under the
 * side before them.
 */
export function trackGroups(tracks: Track[], sides: AlbumSide[] | null | undefined): TrackGroup[] {
  const multiDisc = new Set(tracks.map((t) => t.discNumber || 1)).size > 1;
  const sideOf = new Map<string, AlbumSide>();
  for (const s of sides ?? []) for (const id of s.trackIds) sideOf.set(id, s);
  const out: TrackGroup[] = [];
  const shown = new Set<AlbumSide>();
  let disc: number | null = null;
  let side: AlbumSide | null = null;
  for (const t of tracks) {
    const d = t.discNumber || 1;
    const s: AlbumSide | null = sideOf.get(t.id) ?? side;
    const newDisc = multiDisc && d !== disc;
    const newSide = !!s && s !== side;
    if (!out.length || newDisc || newSide) {
      out.push({ disc: newDisc ? d : null, side: s && !shown.has(s) ? s : null, tracks: [] });
      if (s) shown.add(s);
    }
    out[out.length - 1].tracks.push(t);
    disc = d;
    side = s;
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

/**
 * The release date to show: MusicBrainz's original date when it has the full
 * day, else Plex's.
 */
export function releaseDateFor(plex: string | null | undefined, mb: Pick<AlbumCredits, "released"> | null | undefined): string | null {
  return mb?.released && formatReleaseDate(mb.released) ? mb.released : (plex ?? null);
}

/** Album credit roles in the order they're listed, with their headings. */
export const CREDIT_ROLES: { role: string; label: string }[] = [
  { role: "producer", label: "Produced by" },
  { role: "writer", label: "Written by" },
  { role: "composer", label: "Composed by" },
  { role: "lyricist", label: "Lyrics by" },
  { role: "mixer", label: "Mixed by" },
  { role: "engineer", label: "Engineered by" },
  { role: "mastering", label: "Mastered by" },
];

/** Album credits grouped by role, people in the core's order. Featured artists stay with their songs. */
export function albumCreditGroups(c: Pick<AlbumCredits, "credits" | "state">): { role: string; label: string; people: AlbumCreditRow[] }[] {
  if (c.state !== "found") return [];
  return CREDIT_ROLES.map(({ role, label }) => ({ role, label, people: c.credits.filter((p) => p.roles.includes(role)) })).filter(
    (g) => g.people.length > 0,
  );
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
