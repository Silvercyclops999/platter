/**
 * The words and shapes behind the Stats page's charts: one plain sentence
 * per chart written from the numbers, the span worth drawing (from the
 * first play, not the empty days before it), the peak to highlight, a light
 * minutes scale, and the month calendars. Pure, so tests check them.
 */
import { WEEKDAYS, addDays, hourLabel, minutes, monthName, parseDay, shortDate } from "./stats";
import type { StatsOverview, StatsPoint } from "./statsTypes";

const MONTH_SHORT = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

export const dayCount = (from: string, to: string) => Math.round((parseDay(to).getTime() - parseDay(from).getTime()) / 86_400_000) + 1;

/** "45 min", "1 hr", "2 hrs 10 min", "38 hrs" (minutes drop off at ten hours). */
export function duration(ms: number): string {
  const m = minutes(ms);
  if (m < 60) return `${m} min`;
  const h = Math.floor(m / 60);
  const rest = m % 60;
  const hrs = `${h.toLocaleString()} ${h === 1 ? "hr" : "hrs"}`;
  return h >= 10 || rest === 0 ? hrs : `${hrs} ${rest} min`;
}

/** "Oct 14" when the day is in `year`, "Oct 14, 2025" otherwise. */
export function dayLabel(day: string, year: string): string {
  const d = parseDay(day);
  const short = `${MONTH_SHORT[d.getMonth()]} ${d.getDate()}`;
  return day.startsWith(year) ? short : `${short}, ${d.getFullYear()}`;
}

/** A month key ("YYYY-MM") as "March", or "March 2025" outside `year`. */
function monthWord(month: string, year: string): string {
  const name = monthName(month);
  return month.startsWith(year) ? name.split(" ")[0] : name;
}

// ---------------------------------------------------------------- the span to draw

export interface ActiveSlice {
  points: StatsPoint[];
  /** The first point drawn, when the empty points before it were left out. */
  croppedFrom: string | null;
  /** How much of the range was left out, 0 to 1. */
  croppedShare: number;
}

/** The points from the first one with listening to the end of the range. */
export function activeSlice(points: StatsPoint[]): ActiveSlice {
  const i = points.findIndex((p) => p.ms > 0);
  if (i < 0) return { points: [], croppedFrom: null, croppedShare: 0 };
  return { points: points.slice(i), croppedFrom: i > 0 ? points[i].key : null, croppedShare: points.length ? i / points.length : 0 };
}

/** The point with the most listening (the latest when tied), or null. */
export function bestPoint(points: StatsPoint[]): StatsPoint | null {
  let best: StatsPoint | null = null;
  for (const p of points) if (p.ms > 0 && (!best || p.ms >= best.ms)) best = p;
  return best;
}

// ---------------------------------------------------------------- scales

export interface MinuteScale {
  /** Gridline values in minutes, from 0 up to the top. */
  ticks: number[];
  top: number;
}

const STEPS = [5, 10, 15, 20, 30, 60, 120, 180, 240, 300, 600, 1200, 1800, 3000, 6000, 12000, 30000];

/** A light scale for minutes: at most four steps above zero, on round numbers. */
export function minuteScale(maxMs: number): MinuteScale {
  const max = Math.max(1, minutes(maxMs));
  const step = STEPS.find((s) => Math.ceil(max / s) <= 4) ?? Math.ceil(max / 4);
  const n = Math.max(1, Math.ceil(max / step));
  return { ticks: Array.from({ length: n + 1 }, (_, i) => i * step), top: n * step };
}

/** "30 min", "1 hr", "20 hrs". */
export function tickLabel(min: number): string {
  if (min === 0) return "0";
  if (min < 60) return `${min} min`;
  const h = min / 60;
  return `${Number.isInteger(h) ? h : h.toFixed(1)} ${h === 1 ? "hr" : "hrs"}`;
}

/** Which points get a date under the bars, so labels never crowd. */
export function axisTicks(points: StatsPoint[], bucket: "day" | "month"): number[] {
  const n = points.length;
  if (!n) return [];
  if (bucket === "month") {
    const every = n > 12 ? 3 : n > 6 ? 2 : 1;
    return points.map((_, i) => i).filter((i) => (n - 1 - i) % every === 0);
  }
  if (n <= 10) return points.map((_, i) => i);
  // Sundays, thinned to every other week for longer spans.
  const sundays = points.map((p, i) => (parseDay(p.key).getDay() === 0 ? i : -1)).filter((i) => i >= 0);
  const every = n > 63 ? 4 : n > 35 ? 2 : 1;
  return sundays.filter((_, k) => (sundays.length - 1 - k) % every === 0);
}

// ---------------------------------------------------------------- your day

export interface PeakHours {
  /** First hour of the window (0 to 23). */
  start: number;
  /** The hours in it, in order (wrapping past midnight). */
  hours: number[];
  /** Its share of the day's listening, 0 to 1. */
  share: number;
}

/** The `width` hours in a row (round the clock) with the most listening. */
export function peakHours(hours: number[], width = 3): PeakHours | null {
  const total = hours.reduce((s, h) => s + h, 0);
  if (total <= 0) return null;
  let best = { start: 0, sum: -1 };
  for (let s = 0; s < 24; s++) {
    let sum = 0;
    for (let k = 0; k < width; k++) sum += hours[(s + k) % 24] ?? 0;
    if (sum > best.sum) best = { start: s, sum };
  }
  return { start: best.start, hours: Array.from({ length: width }, (_, k) => (best.start + k) % 24), share: best.sum / total };
}

function listenerKind(peak: PeakHours): string {
  const mid = (peak.start + Math.floor(peak.hours.length / 2)) % 24;
  if (mid >= 22 || mid < 4) return "a night listener";
  if (mid < 12) return "a morning listener";
  if (mid < 17) return "an afternoon listener";
  return "an evening listener";
}

/** The weekday with clearly the most listening (a fifth above the rest's average), or null. */
export function standoutWeekday(weekdays: number[]): number | null {
  const max = Math.max(0, ...weekdays);
  if (max <= 0) return null;
  const i = weekdays.indexOf(max);
  const others = weekdays.filter((_, k) => k !== i);
  const avg = others.reduce((s, v) => s + v, 0) / others.length;
  return max >= avg * 1.2 ? i : null;
}

/** The "Your day" sentence. */
export function daySentence(hours: number[], weekdays: number[], activeDays: number): string {
  const peak = peakHours(hours);
  if (!peak) return "Nothing played in this range yet, so there's no day to show.";
  const span = `between ${hourLabel(peak.start)} and ${hourLabel((peak.start + peak.hours.length) % 24)}`;
  const wd = activeDays >= 5 ? standoutWeekday(weekdays) : null;
  const tail = wd != null ? `, and ${WEEKDAYS[wd]} is your biggest day` : "";
  if (activeDays < 3) return `You've only listened on ${activeDays === 1 ? "one day" : "two days"} here, mostly ${span}.`;
  if (peak.share < 0.3) return `Your listening is spread across the day, with a small peak ${span}${tail}.`;
  const lead = peak.share >= 0.5 ? `most of your music happens ${span}` : `your music peaks ${span}`;
  const kind = listenerKind(peak);
  return `You're ${kind}: ${lead}${tail}.`;
}

// ---------------------------------------------------------------- listening days

/** The run of days in a row that ends on `last`, as its first day. */
function runStart(days: Set<string>, last: string): string {
  let d = last;
  while (days.has(addDays(d, -1))) d = addDays(d, -1);
  return d;
}

/**
 * The listening days sentence. `days` is how many days had plays in the
 * range; `points` are daily points for the range (or its latest part).
 */
export function daysSentence(r: { from: string; to: string; days: number; points: StatsPoint[] }): string {
  const n = dayCount(r.from, r.to);
  const d = r.days;
  const year = r.to.slice(0, 4);
  const on = r.points.filter((p) => p.ms > 0).map((p) => p.key);
  if (d === 0) return "Nothing played in this range yet.";
  if (n === 1) return `You listened on ${dayLabel(r.from, year)}.`;
  if (d >= n) return n <= 7 ? `You listened every one of these ${n} days.` : `You listened every day, ${n.toLocaleString()} in a row.`;
  const set = new Set(on);
  const last = on[on.length - 1];
  if (n <= 35) {
    let s = `You listened on ${d} of these ${n} days`;
    if (last && last >= addDays(r.to, -1)) {
      const start = runStart(set, last);
      const run = dayCount(start, last);
      if (run >= 3) s += `, and every day since ${dayLabel(start, year)}`;
      else if (on[0] > addDays(r.from, 7)) s += `, all of them since ${dayLabel(on[0], year)}`;
    } else if (last && d > 1) {
      s += `, the last time on ${dayLabel(last, year)}`;
    }
    return `${s}.`;
  }
  const recent = on.filter((k) => k > addDays(r.to, -28)).length;
  if (d >= 4 && recent / d >= 0.6) return `You listened on ${d} days, most of them in the last four weeks.`;
  const perWeek = Math.round((d / n) * 7);
  if (d / n >= 0.5) return perWeek >= 7 ? `You listened on ${d.toLocaleString()} of ${n.toLocaleString()} days, nearly every day.` : `You listened on ${d.toLocaleString()} of ${n.toLocaleString()} days, about ${perWeek} days a week.`;
  const months = new Map<string, number>();
  for (const k of on) months.set(k.slice(0, 7), (months.get(k.slice(0, 7)) ?? 0) + 1);
  const top = [...months.entries()].sort((a, b) => b[1] - a[1] || b[0].localeCompare(a[0]));
  if (top[0] && top[0][1] / on.length >= 0.5 && on.length >= 4) return `You listened on ${d} days, most of them in ${monthWord(top[0][0], year)}.`;
  if (top[1] && (top[0][1] + top[1][1]) / on.length >= 0.6 && on.length >= 4) {
    const [a, b] = [top[0][0], top[1][0]].sort();
    return `You listened on ${d} days, mostly in ${monthWord(a, year)} and ${monthWord(b, year)}.`;
  }
  return `You listened on ${d} of ${n.toLocaleString()} days, about one day in ${Math.round(n / d)}.`;
}

// ---------------------------------------------------------------- minutes

/** The minutes chart sentence; `album` is the best day's most played album. */
export function minutesSentence(points: StatsPoint[], bucket: "day" | "month", year: string, album?: { title: string; share: number } | null): string {
  const on = points.filter((p) => p.ms > 0);
  const best = bestPoint(points);
  if (!best) return "No minutes to show for this range yet.";
  if (bucket === "month") {
    if (on.length === 1) return `All of it was in ${monthWord(best.key, year)}: ${duration(best.ms)}.`;
    return `Your biggest month was ${monthWord(best.key, year)}, with ${duration(best.ms)}.`;
  }
  const wd = WEEKDAYS[parseDay(best.key).getDay()];
  const what = album ? (album.share >= 0.5 ? `, most of it ${album.title}` : `, more ${album.title} than anything else`) : "";
  if (on.length === 1) return `You listened on one day, ${wd}, ${dayLabel(best.key, year)}, for ${duration(best.ms)}${what}.`;
  return `Your biggest day was ${wd}, ${dayLabel(best.key, year)}: ${duration(best.ms)}${what}.`;
}

/** The line under the minutes chart: the average on days (or months) you listened, and the total. */
export function minutesFootnote(points: StatsPoint[], bucket: "day" | "month"): string {
  const on = points.filter((p) => p.ms > 0);
  if (!on.length) return "";
  const total = on.reduce((s, p) => s + p.ms, 0);
  return `${duration(total / on.length)} on an average ${bucket} you listened. ${duration(total)} in all.`;
}

// ---------------------------------------------------------------- the hero

/** The sentence at the top: how long, on how many days, and who took the most of it. */
export function heroSentence(o: StatsOverview): { time: string; days: string; artist: string | null; share: string | null } {
  const t = o.totals;
  const a = o.topArtists[0];
  const pct = a && t.ms > 0 ? a.ms / t.ms : 0;
  const share = !a ? null : pct >= 0.6 ? "most of it" : pct >= 0.45 ? "about half of it" : pct >= 0.3 ? "about a third of it" : pct >= 0.2 ? "about a quarter of it" : `${Math.max(1, Math.round(pct * 100))}% of it, more than anyone else`;
  return { time: duration(t.ms), days: `${t.days.toLocaleString()} ${t.days === 1 ? "day" : "days"}`, artist: a?.name ?? null, share };
}

// ---------------------------------------------------------------- the calendar

export interface MonthCell {
  day: string;
  date: number;
  ms: number;
  plays: number;
  /** 0 is no listening, 1 to 3 more; the best day is drawn apart. */
  level: 0 | 1 | 2 | 3;
  inRange: boolean;
}

export interface CalendarMonth {
  month: string;
  label: string;
  /** Rows of seven, Sunday first; null pads the first and last week. */
  weeks: (MonthCell | null)[][];
  days: number;
  ms: number;
}

/**
 * Month calendars from the month of the first play in the range to the
 * range's last month (at most `cap`, the latest). Each day's level scales
 * to the busiest day shown.
 */
export function calendarMonths(points: StatsPoint[], from: string, to: string, cap = 12): { months: CalendarMonth[]; best: MonthCell | null; capped: boolean } {
  const by = new Map(points.map((p) => [p.key, p]));
  const firstOn = points.find((p) => p.ms > 0 && p.key >= from && p.key <= to);
  if (!firstOn) return { months: [], best: null, capped: false };
  const keys: string[] = [];
  let [y, m] = firstOn.key.split("-").map(Number);
  const [ey, em] = to.split("-").map(Number);
  while (y < ey || (y === ey && m <= em)) {
    keys.push(`${y}-${String(m).padStart(2, "0")}`);
    [y, m] = m === 12 ? [y + 1, 1] : [y, m + 1];
  }
  const shown = keys.slice(-cap);
  const max = Math.max(0, ...points.filter((p) => p.key >= `${shown[0]}-01` && p.key <= to).map((p) => p.ms));
  let best: MonthCell | null = null;
  const year = to.slice(0, 4);
  const months = shown.map((key): CalendarMonth => {
    const [yy, mm] = key.split("-").map(Number);
    const len = new Date(yy, mm, 0).getDate();
    const lead = new Date(yy, mm - 1, 1).getDay();
    const cells: (MonthCell | null)[] = Array.from({ length: lead }, () => null);
    let days = 0;
    let ms = 0;
    for (let i = 1; i <= len; i++) {
      const day = `${key}-${String(i).padStart(2, "0")}`;
      const p = by.get(day);
      const inRange = day >= from && day <= to;
      const v = inRange ? (p?.ms ?? 0) : 0;
      const level = v <= 0 || max <= 0 ? 0 : (Math.min(3, 1 + Math.floor(2.999 * Math.sqrt(v / max))) as MonthCell["level"]);
      const cell: MonthCell = { day, date: i, ms: v, plays: inRange ? (p?.plays ?? 0) : 0, level, inRange };
      if (v > 0) {
        days++;
        ms += v;
        if (!best || v >= best.ms) best = cell;
      }
      cells.push(cell);
    }
    while (cells.length % 7) cells.push(null);
    const weeks = Array.from({ length: cells.length / 7 }, (_, w) => cells.slice(w * 7, w * 7 + 7));
    return { month: key, label: monthWord(key, year), weeks, days, ms };
  });
  return { months, best, capped: keys.length > cap };
}

/** A calendar day in words, for its label and the readout. */
export function calendarDayLabel(c: MonthCell): string {
  const wd = WEEKDAYS[parseDay(c.day).getDay()].slice(0, 3);
  if (!c.inRange) return `${wd}, ${shortDate(c.day)}: outside this range`;
  return `${wd}, ${shortDate(c.day)}: ${c.ms > 0 ? `${duration(c.ms)}, ${c.plays.toLocaleString()} ${c.plays === 1 ? "play" : "plays"}` : "no listening"}`;
}

export interface ActiveWeek {
  /** The week's Sunday. */
  start: string;
  cells: MonthCell[];
  /** Weeks with no listening skipped just before this one. */
  quietBefore: number;
}

/**
 * Only the weeks with listening, from the first play to `to`, for a range
 * that's mostly empty. Each week remembers how many quiet weeks it skipped.
 */
export function activeWeeks(points: StatsPoint[], from: string, to: string): { weeks: ActiveWeek[]; best: MonthCell | null } {
  const by = new Map(points.filter((p) => p.key >= from && p.key <= to).map((p) => [p.key, p]));
  const on = [...by.values()].filter((p) => p.ms > 0).map((p) => p.key).sort();
  if (!on.length) return { weeks: [], best: null };
  const max = Math.max(...[...by.values()].map((p) => p.ms));
  const weeks: ActiveWeek[] = [];
  let best: MonthCell | null = null;
  let quiet = 0;
  for (let start = addDays(on[0], -parseDay(on[0]).getDay()); start <= to; start = addDays(start, 7)) {
    const cells = Array.from({ length: 7 }, (_, i): MonthCell => {
      const day = addDays(start, i);
      const inRange = day >= from && day <= to;
      const p = inRange ? by.get(day) : undefined;
      const v = p?.ms ?? 0;
      const level = v <= 0 ? 0 : (Math.min(3, 1 + Math.floor(2.999 * Math.sqrt(v / max))) as MonthCell["level"]);
      return { day, date: parseDay(day).getDate(), ms: v, plays: p?.plays ?? 0, level, inRange };
    });
    if (!cells.some((c) => c.ms > 0)) {
      quiet++;
      continue;
    }
    for (const c of cells) if (c.ms > 0 && (!best || c.ms >= best.ms)) best = c;
    weeks.push({ start, cells, quietBefore: weeks.length ? quiet : 0 });
    quiet = 0;
  }
  return { weeks, best };
}

/** Whether a span is mostly empty: over eight weeks with listening on under a quarter of its days. */
export function isMostlyEmpty(points: StatsPoint[], to: string): boolean {
  const on = points.filter((p) => p.ms > 0 && p.key <= to);
  if (!on.length) return false;
  const span = dayCount(on[0].key, to);
  return span > 56 && on.length / span < 0.25;
}

/** The order hours are drawn in: from 6 AM, unless the peak runs across 6 AM, then from midnight. */
export function hourOrder(peak: PeakHours | null): number[] {
  const start = peak && peak.hours.includes(6) && peak.hours[0] !== 6 ? 0 : 6;
  return Array.from({ length: 24 }, (_, i) => (start + i) % 24);
}
