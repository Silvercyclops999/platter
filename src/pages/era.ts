/** Eras and versions: decisions kept apart from the components so they can be tested. */
import type { ArtistPage, Era, SongStatus, Track } from "../lib/types";

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/** "2019", "Feb 2019", "Nov 2, 2018" from an ARCHIVE_DATE in a pinned form; anything else as written. */
export function formatArchiveDate(d: string): string {
  const m = /^(\d{4})(?:-(\d{2}))?(?:-(\d{2}))?$/.exec(d.trim());
  if (!m) return d;
  const [, y, mo, day] = m;
  if (!mo) return y;
  const month = MONTHS[Number(mo) - 1];
  if (!month) return y;
  return day ? `${month} ${Number(day)}, ${y}` : `${month} ${y}`;
}

/** An era's dates for its card: "2019", "Nov 2018 to Feb 2019", or null. */
export function eraRange(e: Pick<Era, "start" | "end">): string | null {
  const short = (d: string) => {
    const m = /^(\d{4})(?:-(\d{2}))?/.exec(d);
    if (!m) return d;
    return m[2] && MONTHS[Number(m[2]) - 1] ? `${MONTHS[Number(m[2]) - 1]} ${m[1]}` : m[1];
  };
  if (!e.start) return null;
  const a = short(e.start);
  const b = e.end ? short(e.end) : a;
  return a === b ? a : `${a} to ${b}`;
}

/** Era cards show once an artist has two or more eras (and the setting is on). */
export function showEraCards(eras: Era[] | undefined, on: boolean | undefined): boolean {
  return on !== false && (eras?.length ?? 0) >= 2;
}

/**
 * The artist page filtered to one era: Popular, every section and Appears on
 * keep only what the era holds, released and unreleased alike. Sections
 * left empty go.
 */
export function filterByEra(page: ArtistPage, era: Era | null): ArtistPage {
  if (!era) return page;
  const albums = new Set(era.albumIds);
  const tracks = new Set(era.trackIds);
  return {
    ...page,
    popular: page.popular.filter((t) => tracks.has(t.id)),
    popularElsewhere: [],
    sections: page.sections.map((s) => ({ ...s, albums: s.albums.filter((a) => albums.has(a.id)) })).filter((s) => s.albums.length),
    appearsOn: page.appearsOn.filter((a) => albums.has(a.id)),
  };
}

/** Whether a row shows "+N versions": a released song with others, while the setting is on. */
export function showVersionLink(t: Pick<Track, "otherVersions" | "songStatus">, on: boolean | undefined): boolean {
  return on !== false && (t.otherVersions ?? 0) > 0 && t.songStatus === "released";
}

/** "+1 version", "+3 versions". */
export function versionLinkText(n: number): string {
  return `+${n} ${n === 1 ? "version" : "versions"}`;
}

/** "1 other version", "4 other versions" (search). */
export function otherVersionsText(n: number): string {
  return `${n} other ${n === 1 ? "version" : "versions"}`;
}

/** Words for a song's own status in the versions panel. */
export const SONG_STATUS_LABEL: Record<SongStatus, string> = {
  released: "Released",
  unreleased: "Unreleased",
  probablyUnreleased: "Probably unreleased",
  probablyAlternate: "Probably an alternate version",
};
