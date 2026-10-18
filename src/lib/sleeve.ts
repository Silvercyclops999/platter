/**
 * Words for sleeve notes: role headings, source names, the facts' labels
 * and dates as people write them on a sleeve.
 */
import type { SleeveFact, SleeveNotes, SleeveSource } from "./types";

/** "Produced by" and friends, in the order the core groups them. */
export const ROLE_LABEL: Record<string, string> = {
  producer: "Produced by",
  writer: "Written by",
  composer: "Composed by",
  lyricist: "Lyrics by",
  arranger: "Arranged by",
  performer: "Musicians",
  featured: "Featuring",
  conductor: "Conducted by",
  engineer: "Engineered by",
  mixer: "Mixed by",
  mastering: "Mastered by",
};

/** A role's heading; unknown roles read as themselves. */
export const roleLabel = (role: string): string => ROLE_LABEL[role] ?? role.charAt(0).toUpperCase() + role.slice(1);

/** Credit page headings ("Produced", "Played on"). */
export const ROLE_PAGE_LABEL: Record<string, string> = {
  producer: "Produced",
  writer: "Written",
  composer: "Composed",
  lyricist: "Lyrics",
  arranger: "Arranged",
  performer: "Played on",
  featured: "Featured on",
  conductor: "Conducted",
  engineer: "Engineered",
  mixer: "Mixed",
  mastering: "Mastered",
};

export const SOURCE_NAME: Record<SleeveSource, string> = {
  tags: "your files' tags",
  plex: "Plex",
  musicbrainz: "MusicBrainz",
  discogs: "Discogs",
};

/** The short mark printed next to a fact. */
export const SOURCE_MARK: Record<SleeveSource, string> = {
  tags: "Tags",
  plex: "Plex",
  musicbrainz: "MB",
  discogs: "Discogs",
};

/** "your files' tags and MusicBrainz". */
export function sourcesText(sources: SleeveSource[]): string {
  const names = sources.map((s) => SOURCE_NAME[s]);
  if (names.length <= 1) return names[0] ?? "";
  return `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}`;
}

const MONTHS = { year: "numeric", month: "long" } as const;
const DAYS = { year: "numeric", month: "long", day: "numeric" } as const;

/** One date as written: "1997", "May 1997", "May 21, 1997". */
export function formatDay(d: string, locale?: string): string {
  const m = /^(\d{4})(?:-(\d{2}))?(?:-(\d{2}))?/.exec(d.trim());
  if (!m) return d;
  const [y, mo, day] = [Number(m[1]), m[2] ? Number(m[2]) : 0, m[3] ? Number(m[3]) : 0];
  if (!mo || mo > 12) return String(y);
  const date = new Date(Date.UTC(y, mo - 1, day && day <= 31 ? day : 1));
  return date.toLocaleDateString(locale, { ...(day ? DAYS : MONTHS), timeZone: "UTC" });
}

/** A date or a span ("1996-11/1997-02") as written: "November 1996 to February 1997". */
export function formatSleeveDate(v: string, locale?: string): string {
  const [a, b] = v.split("/");
  if (!b || a === b) return formatDay(a, locale);
  return `${formatDay(a, locale)} to ${formatDay(b, locale)}`;
}

/** A country code as a name ("GB" reads "United Kingdom"); "XW" is worldwide. */
export function countryName(code: string, locale?: string): string {
  const c = code.trim().toUpperCase();
  if (c === "XW") return "Worldwide";
  if (c === "XE") return "Europe";
  if (!/^[A-Z]{2}$/.test(c)) return code;
  try {
    return new Intl.DisplayNames(locale ? [locale] : undefined, { type: "region" }).of(c) ?? code;
  } catch {
    return code;
  }
}

const STATUS_TEXT: Record<string, string> = {
  released: "Released",
  unreleased: "Unreleased",
  leaked: "Leaked",
  alternate: "Alternate versions",
};

export interface FactRow {
  key: string;
  label: string;
  text: string;
  fact: SleeveFact;
}

/** The facts a sleeve prints, in order, as label and text. Missing ones are left out. */
export function factRows(n: SleeveNotes, locale?: string): FactRow[] {
  const rows: FactRow[] = [];
  const add = (key: string, label: string, fact: SleeveFact | null, text?: (v: string) => string) => {
    if (fact) rows.push({ key, label, fact, text: text ? text(fact.value) : fact.value });
  };
  add("released", "Released", n.released, (v) => formatSleeveDate(v, locale));
  add("editionDate", "This edition", n.editionDate, (v) => formatSleeveDate(v, locale));
  add("recorded", "Recorded", n.recorded, (v) => formatSleeveDate(v, locale));
  add("label", "Label", n.label);
  add("catalogue", "Catalogue no.", n.catalogue);
  add("format", "Format", n.format);
  add("edition", "Edition", n.edition);
  add("country", "Country", n.country, (v) => countryName(v, locale));
  add("barcode", "Barcode", n.barcode);
  add("status", "Status", n.status, (v) => STATUS_TEXT[v] ?? v);
  add("origin", "Source", n.origin);
  return rows;
}

/** Studio lines: "Recorded at", "Mixed at", "Mastered at". */
export const STUDIO_LABEL: Record<string, string> = { recorded: "Recorded at", mixed: "Mixed at", mastered: "Mastered at" };
