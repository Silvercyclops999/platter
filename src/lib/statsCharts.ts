/**
 * Stats page charts as plain data: the listening calendar's window and
 * cells, the 24-hour clock's spokes, chart averages and the words each
 * chart gives a screen reader. Pure, so tests check them.
 */
import { addDays, formatMinutes, hourLabel, minutes, monthName, parseDay, shortDate } from "./stats";
import type { StatsPoint } from "./statsTypes";

/** Days the calendar shows, both ends included, starting on a Sunday. */
export interface CalendarWindow {
  /** First day drawn (a Sunday). */
  from: string;
  /** Last day drawn (a Saturday). */
  to: string;
  /** What the calendar's heading says. */
  title: string;
}

const dayCount = (from: string, to: string) => Math.round((parseDay(to).getTime() - parseDay(from).getTime()) / 86_400_000);

/** The Sunday on or before a day. */
export function weekStart(day: string): string {
  return addDays(day, -parseDay(day).getDay());
}

/**
 * The calendar for a range ("YYYY-MM-DD", both included): the whole year
 * when the range starts on January 1st and stays in that year (a year, or
 * this year so far), otherwise the 53 weeks that end with the range's last
 * week. Always whole weeks, at most 371 days.
 */
export function calendarWindow(from: string, to: string): CalendarWindow {
  const year = from.slice(0, 4);
  if (from.endsWith("-01-01") && to.slice(0, 4) === year) {
    return { from: weekStart(`${year}-01-01`), to: addDays(weekStart(`${year}-12-31`), 6), title: year };
  }
  const end = addDays(weekStart(to), 6);
  const start = addDays(weekStart(to), -52 * 7);
  return { from: start, to: end, title: `${shortDate(start)} to ${shortDate(end)}` };
}

export interface CalendarCell {
  day: string;
  ms: number;
  plays: number;
  /** 0 is no listening, 4 the most. */
  level: 0 | 1 | 2 | 3 | 4;
  /** Inside the range being looked at. */
  inRange: boolean;
  /** Inside the calendar's own year or window (false for padding days). */
  inWindow: boolean;
  /** After the range's last day (the rest of this year, or this week). */
  after: boolean;
}

export interface CalendarGrid {
  /** Columns of seven days, Sunday first. */
  weeks: CalendarCell[][];
  /** Month labels: the week column each month starts in. */
  months: { label: string; week: number }[];
  /** Days with plays inside the range. */
  daysListened: number;
  busiest: CalendarCell | null;
}

/** A listening level from 1 to 4, on a square-root scale so quiet days still show. */
export function level(ms: number, max: number): CalendarCell["level"] {
  if (ms <= 0 || max <= 0) return 0;
  return Math.min(4, 1 + Math.floor(3.999 * Math.sqrt(ms / max))) as CalendarCell["level"];
}

const MONTH_SHORT = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/**
 * The calendar's cells. Levels scale to the busiest day in the range; days
 * outside the range keep their level but draw quieter. A year's padding
 * days (the last of December before, the first of January after) stay empty.
 */
export function calendarGrid(w: CalendarWindow, points: StatsPoint[], range: { from: string; to: string }): CalendarGrid {
  const by = new Map(points.map((p) => [p.key, p]));
  const isYear = /^\d{4}$/.test(w.title);
  const inRange = (d: string) => d >= range.from && d <= range.to;
  const max = Math.max(0, ...points.filter((p) => inRange(p.key)).map((p) => p.ms));
  const n = dayCount(w.from, w.to) + 1;
  const weeks: CalendarCell[][] = [];
  const months: CalendarGrid["months"] = [];
  let busiest: CalendarCell | null = null;
  let daysListened = 0;
  for (let i = 0; i < n; i++) {
    const day = addDays(w.from, i);
    const inWindow = !isYear || day.startsWith(w.title);
    const p = inWindow ? by.get(day) : undefined;
    const cell: CalendarCell = { day, ms: p?.ms ?? 0, plays: p?.plays ?? 0, level: level(p?.ms ?? 0, max), inRange: inWindow && inRange(day), inWindow, after: day > range.to };
    if (i % 7 === 0) weeks.push([]);
    weeks[weeks.length - 1].push(cell);
    if (inWindow && day.slice(8) === "01") months.push({ label: MONTH_SHORT[Number(day.slice(5, 7)) - 1], week: weeks.length - 1 });
    if (cell.inRange && cell.ms > 0) {
      daysListened++;
      if (!busiest || cell.ms > busiest.ms) busiest = cell;
    }
  }
  return { weeks, months, daysListened, busiest };
}

/** What the calendar says to a screen reader. */
export function calendarSummary(w: CalendarWindow, g: CalendarGrid): string {
  const best = g.busiest ? ` Busiest day: ${shortDate(g.busiest.day)}, ${formatMinutes(g.busiest.ms)}.` : "";
  return `Listening calendar, ${w.title}. ${g.daysListened.toLocaleString()} ${g.daysListened === 1 ? "day" : "days"} with listening in this range.${best}`;
}

/** The calendar as rows for its text table: each month with listening days and minutes. */
export function calendarMonths(g: CalendarGrid): { month: string; days: number; minutes: number }[] {
  const by = new Map<string, { days: number; ms: number }>();
  for (const c of g.weeks.flat()) {
    if (!c.inRange) continue;
    const k = c.day.slice(0, 7);
    const t = by.get(k) ?? { days: 0, ms: 0 };
    if (c.ms > 0) t.days++;
    t.ms += c.ms;
    by.set(k, t);
  }
  return [...by.entries()].map(([k, t]) => ({ month: monthName(k), days: t.days, minutes: minutes(t.ms) }));
}

/** One spoke of the 24-hour clock, in a box `size` wide. */
export interface ClockSpoke {
  hour: number;
  x1: number;
  y1: number;
  x2: number;
  y2: number;
  /** 0 to 1 of the busiest hour. */
  fraction: number;
}

/**
 * Spokes for listening by hour: midnight at the top, going clockwise, each
 * as long as its share of the busiest hour (a short stub when silent).
 */
export function clockSpokes(hours: number[], size: number, inner: number, outer: number): ClockSpoke[] {
  const max = Math.max(0, ...hours);
  const c = size / 2;
  return hours.map((ms, hour) => {
    const fraction = max > 0 ? ms / max : 0;
    const a = (hour / 24) * Math.PI * 2 - Math.PI / 2 + Math.PI / 24;
    const r2 = inner + Math.max(2, fraction * (outer - inner));
    const round = (v: number) => Math.round(v * 10) / 10;
    return { hour, x1: round(c + inner * Math.cos(a)), y1: round(c + inner * Math.sin(a)), x2: round(c + r2 * Math.cos(a)), y2: round(c + r2 * Math.sin(a)), fraction };
  });
}

/** What the clock says to a screen reader. */
export function clockSummary(hours: number[], busiest: number | null): string {
  if (busiest == null) return "Listening by hour of the day: nothing yet.";
  const total = hours.reduce((s, h) => s + h, 0) || 1;
  const parts = [
    ["night (12 AM to 6 AM)", 0, 6],
    ["morning (6 AM to 12 PM)", 6, 12],
    ["afternoon (12 PM to 6 PM)", 12, 18],
    ["evening (6 PM to 12 AM)", 18, 24],
  ] as const;
  const shares = parts.map(([name, a, b]) => `${name} ${Math.round((hours.slice(a, b).reduce((s, h) => s + h, 0) / total) * 100)}%`);
  return `Listening by hour of the day. Busiest hour: ${hourLabel(busiest)}. ${shares.join(", ")}.`;
}

/** The average of the points with listening, or 0. */
export function averageMs(points: StatsPoint[]): number {
  const on = points.filter((p) => p.ms > 0);
  return on.length ? on.reduce((s, p) => s + p.ms, 0) / on.length : 0;
}

/** What the bar chart says to a screen reader. */
export function seriesSummary(points: StatsPoint[], bucket: "day" | "month", label: (p: StatsPoint) => string): string {
  const on = points.filter((p) => p.ms > 0);
  if (!on.length) return `Minutes listened per ${bucket}: nothing in this range.`;
  const best = on.reduce((a, b) => (b.ms > a.ms ? b : a));
  return `Minutes listened per ${bucket}. ${on.length} of ${points.length} ${bucket}s with listening, averaging ${minutes(averageMs(points)).toLocaleString()} minutes. Most: ${label(best)}, ${formatMinutes(best.ms)}.`;
}

/** Each row's bar: its share of the first (top) row, 0 to 1. */
export function shareOfTop(values: number[]): number[] {
  const top = values[0] ?? 0;
  return values.map((v) => (top > 0 ? Math.min(1, v / top) : 0));
}
