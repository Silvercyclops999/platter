/** Stats page helpers: range choices, local dates and number wording. Pure, so tests check them. */
import type { StatsRange } from "./statsTypes";

export type RangeChoice =
  | { kind: "4w" }
  | { kind: "6m" }
  | { kind: "year" }
  | { kind: "pickYear"; year: number }
  | { kind: "pickMonth"; month: string }
  | { kind: "all" }
  | { kind: "custom"; from: string; to: string };

const pad = (n: number) => String(n).padStart(2, "0");

/** A local date as "YYYY-MM-DD". */
export function localDay(d: Date): string {
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/** "YYYY-MM-DD" to a local Date at midnight. */
export function parseDay(s: string): Date {
  const [y, m, d] = s.split("-").map(Number);
  return new Date(y, (m ?? 1) - 1, d ?? 1);
}

export function addDays(day: string, n: number): string {
  const d = parseDay(day);
  d.setDate(d.getDate() + n);
  return localDay(d);
}

/** The same day `n` months earlier (clamped to the month's end). */
export function addMonths(day: string, n: number): string {
  const d = parseDay(day);
  const want = d.getMonth() + n;
  const t = new Date(d.getFullYear(), want, 1);
  const last = new Date(t.getFullYear(), t.getMonth() + 1, 0).getDate();
  t.setDate(Math.min(d.getDate(), last));
  return localDay(t);
}

export function monthBounds(month: string): { from: string; to: string } {
  const [y, m] = month.split("-").map(Number);
  const last = new Date(y, m, 0).getDate();
  return { from: `${month}-01`, to: `${month}-${pad(last)}` };
}

/** The days a choice counts, with `today` as "YYYY-MM-DD". */
export function toRange(c: RangeChoice, today: string): StatsRange {
  switch (c.kind) {
    case "4w":
      return { from: addDays(today, -27), to: today };
    case "6m":
      return { from: addDays(addMonths(today, -6), 1), to: today };
    case "year":
      return { from: `${today.slice(0, 4)}-01-01`, to: today };
    case "pickYear":
      return { from: `${c.year}-01-01`, to: `${c.year}-12-31` };
    case "pickMonth":
      return monthBounds(c.month);
    case "all":
      return { from: null, to: null };
    case "custom":
      return c.from <= c.to ? { from: c.from, to: c.to } : { from: c.to, to: c.from };
  }
}

const MONTHS = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
export const WEEKDAYS = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

/** "September 2026". */
export function monthName(month: string): string {
  const [y, m] = month.split("-").map(Number);
  return `${MONTHS[m - 1] ?? ""} ${y}`;
}

/** "Sep 3, 2026". */
export function shortDate(day: string): string {
  const d = parseDay(day);
  return `${MONTHS[d.getMonth()].slice(0, 3)} ${d.getDate()}, ${d.getFullYear()}`;
}

/** What the page says it's showing. */
export function rangeLabel(c: RangeChoice): string {
  switch (c.kind) {
    case "4w":
      return "Last 4 weeks";
    case "6m":
      return "Last 6 months";
    case "year":
      return "This year";
    case "pickYear":
      return String(c.year);
    case "pickMonth":
      return monthName(c.month);
    case "all":
      return "All time";
    case "custom":
      return `${shortDate(c.from <= c.to ? c.from : c.to)} to ${shortDate(c.from <= c.to ? c.to : c.from)}`;
  }
}

/** Whole minutes listened. */
export function minutes(ms: number): number {
  return Math.round(ms / 60_000);
}

export function formatMinutes(ms: number): string {
  const m = minutes(ms);
  return `${m.toLocaleString()} ${m === 1 ? "minute" : "minutes"}`;
}

export function formatPlays(n: number): string {
  return `${n.toLocaleString()} ${n === 1 ? "play" : "plays"}`;
}

/** "9 PM", "12 AM". */
export function hourLabel(h: number): string {
  const n = h % 12 === 0 ? 12 : h % 12;
  return `${n} ${h < 12 ? "AM" : "PM"}`;
}

/** "+12%" / "-8%" / null when there's nothing to compare with. */
export function change(now: number, before: number): string | null {
  if (before <= 0) return null;
  const pct = Math.round(((now - before) / before) * 100);
  return `${pct > 0 ? "+" : ""}${pct}%`;
}

/** Bar heights as fractions of the tallest (0 to 1). */
export function barFractions(values: number[]): number[] {
  const max = Math.max(0, ...values);
  return values.map((v) => (max > 0 ? v / max : 0));
}

/** An SVG polyline path for a sparkline in a `w` by `h` box. */
export function sparkPath(values: number[], w: number, h: number, pad = 2): string {
  if (values.length === 0) return "";
  const max = Math.max(1, ...values);
  const step = values.length > 1 ? (w - pad * 2) / (values.length - 1) : 0;
  return values
    .map((v, i) => {
      const x = pad + i * step;
      const y = h - pad - (v / max) * (h - pad * 2);
      return `${i === 0 ? "M" : "L"}${x.toFixed(1)} ${y.toFixed(1)}`;
    })
    .join(" ");
}

/** Where a first listen happened, as words. */
export function playedFromLabel(from: { kind: string; title: string | null; id?: string | null } | null): string | null {
  if (!from) return null;
  switch (from.kind) {
    case "album":
    case "playlist":
    case "artist":
      return from.title ? `from ${from.title}` : null;
    case "liked":
      return "from Liked Songs";
    case "mix":
      return "from a mix";
    case "radio":
      return "from radio";
    case "search":
      return "from search";
    case "imported":
      switch (from.id) {
        case "plex_history":
          return "from your Plex history";
        case "tautulli":
          return "from Tautulli";
        case "lastfm":
          return "from Last.fm";
        default:
          return "in your imported history";
      }
    default:
      return null;
  }
}
