/** Words for crates: tab names, the lines under a crate's name, shelf titles. */
import type { AlbumShelf, CrateKind } from "../lib/types";

/** The kinds the Crates page has tabs for, in order. Years open from their era. */
export const CRATE_TABS: readonly CrateKind[] = ["label", "producer", "studio", "era", "genre"];

export const CRATE_TAB_LABEL: Record<CrateKind, string> = {
  label: "Labels",
  producer: "Producers",
  studio: "Studios",
  era: "Eras",
  year: "Years",
  genre: "Genres",
};

/** One crate of a kind, and many. */
export const CRATE_NOUN: Record<CrateKind, [string, string]> = {
  label: ["label", "labels"],
  producer: ["producer", "producers"],
  studio: ["studio", "studios"],
  era: ["decade", "decades"],
  year: ["year", "years"],
  genre: ["genre", "genres"],
};

/** The small engraved word over a crate page's name. */
export const CRATE_PLATE: Record<CrateKind, string> = {
  label: "Label",
  producer: "Producer",
  studio: "Studio",
  era: "Era",
  year: "Year",
  genre: "Genre",
};

/** Where each kind's crates come from, said once under the tabs. */
export const CRATE_SOURCE: Record<CrateKind, string> = {
  label: "Labels come from your files' tags, Plex and the sleeve notes you've opened.",
  producer: "Producers come from your files' credit tags and the sleeve notes you've opened.",
  studio: "Studios come from sleeve notes looked up online, so more turn up as you open albums' notes.",
  era: "Each decade holds the records released in it.",
  year: "Each year holds the records released in it.",
  genre: "Genres are the ones Plex files your albums under.",
};

export const records = (n: number) => (n === 1 ? "1 record" : `${n.toLocaleString()} records`);
const artists = (n: number) => (n === 1 ? "one artist" : `${n.toLocaleString()} artists`);

/** "1991 to 2004", "1994", or "" when no album has a year. */
export function yearSpan(from: number | null, to: number | null): string {
  if (!from || !to) return "";
  return from === to ? String(from) : `${from} to ${to}`;
}

/** The line under a crate's name: "14 records by 9 artists, 1991 to 2004". */
export function crateLine(c: { count: number; artists: number; fromYear: number | null; toYear: number | null; kind: CrateKind }): string {
  const span = c.kind === "year" ? "" : yearSpan(c.fromYear, c.toYear);
  const by = c.artists === c.count && c.count > 1 ? `, each by a different artist` : ` by ${artists(c.artists)}`;
  return `${records(c.count)}${by}${span ? `, ${span}` : ""}`;
}

/** The sentence on a crate page, led by what the crate is. */
export function crateSentence(c: { kind: CrateKind; name: string; count: number; artists: number; fromYear: number | null; toYear: number | null }): string {
  const n = c.count;
  const yours = n === 1 ? "1 of your records" : `${n.toLocaleString()} of your records`;
  const span = yearSpan(c.fromYear, c.toYear);
  const when = span && c.kind !== "era" && c.kind !== "year" ? (c.fromYear === c.toYear ? `, in ${span}` : `, from ${span}`) : "";
  const by = `by ${artists(c.artists)}`;
  switch (c.kind) {
    case "label":
      return `${yours} came out on ${c.name}, ${by}${when}.`;
    case "producer":
      return `${c.name} produced ${yours}, for ${artists(c.artists)}${when}.`;
    case "studio":
      return `${yours} ${n === 1 ? "was" : "were"} made at ${c.name}, ${by}${when}.`;
    case "era":
      return `${yours} came out in the ${c.name}, ${by}.`;
    case "year":
      return `${yours} came out in ${c.name}, ${by}.`;
    case "genre":
      return `${yours} ${n === 1 ? "is" : "are"} filed under ${c.name}, ${by}${when}.`;
  }
}

/** An album page shelf's title. */
export function shelfTitle(s: Pick<AlbumShelf, "kind" | "name">): string {
  switch (s.kind) {
    case "label":
      return `More from ${s.name}`;
    case "producer":
      return `More produced by ${s.name}`;
    case "studio":
      return `Also recorded at ${s.name}`;
    default:
      return `More from ${s.name}`;
  }
}

/** Album page shelves shown before the rest fold under "More crates". */
export const SHELVES_SHOWN = 2;
