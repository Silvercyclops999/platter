/** Release radar wording, kept pure so it's easy to test. */
import type { RadarItem } from "./types";

const DAY_MS = 86_400_000;

/** "Album", "EP", "Single". */
export function radarKind(kind: string): string {
  return kind === "ep" ? "EP" : kind === "single" ? "Single" : "Album";
}

/** Midnight UTC of a "YYYY-MM-DD" date (a month or year alone counts from its first day). */
function dayOf(date: string): number | null {
  const m = /^(\d{4})(?:-(\d{2}))?(?:-(\d{2}))?$/.exec(date.trim());
  if (!m) return null;
  return Date.UTC(Number(m[1]), Number(m[2] ?? "1") - 1, Number(m[3] ?? "1"));
}

const monthDay = new Intl.DateTimeFormat("en-US", { month: "short", day: "numeric", timeZone: "UTC" });
const monthYear = new Intl.DateTimeFormat("en-US", { month: "short", year: "numeric", timeZone: "UTC" });

/**
 * When it came out: "Out Feb 3" ahead of time, "Today", "Yesterday",
 * "5 days ago" within two weeks, then "Dec 2". A month alone is "Mar 2027".
 */
export function radarDate(date: string, now = Date.now()): string {
  const at = dayOf(date);
  if (at === null) return date;
  if (/^\d{4}$/.test(date.trim())) return date.trim();
  if (/^\d{4}-\d{2}$/.test(date.trim())) return monthYear.format(at);
  const today = Math.floor(now / DAY_MS) * DAY_MS;
  const days = Math.round((today - at) / DAY_MS);
  if (days < 0) return `Out ${monthDay.format(at)}`;
  if (days === 0) return "Today";
  if (days === 1) return "Yesterday";
  if (days < 14) return `${days} days ago`;
  return monthDay.format(at);
}

export type RadarFilter = "all" | "albums" | "epsSingles";

export const RADAR_FILTERS: { id: RadarFilter; label: string }[] = [
  { id: "all", label: "All" },
  { id: "albums", label: "Albums" },
  { id: "epsSingles", label: "EPs and singles" },
];

export function filterRadar(items: RadarItem[], f: RadarFilter): RadarItem[] {
  if (f === "albums") return items.filter((i) => i.kind === "album");
  if (f === "epsSingles") return items.filter((i) => i.kind !== "album");
  return items;
}

/** Daily limit choices for new albums from followed artists (same as the core's). */
export const AUTO_CAP_OPTIONS = [
  { value: "1", label: "1 a day" },
  { value: "2", label: "2 a day" },
  { value: "3", label: "3 a day" },
  { value: "5", label: "5 a day" },
  { value: "10", label: "10 a day" },
] as const;
