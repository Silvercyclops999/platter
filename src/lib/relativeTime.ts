/** Dates for the library table: "14 hours ago", "3 weeks ago", "Oct 3, 2026". Inputs are unix seconds. */

const MIN = 60;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;
const WEEK = 7 * DAY;

const unit = (n: number, one: string) => `${n} ${one}${n === 1 ? "" : "s"} ago`;

/** How long ago, in the largest whole unit. */
export function timeAgo(at: number, now = Date.now() / 1000): string {
  const d = Math.max(0, now - at);
  if (d < MIN) return "Just now";
  if (d < HOUR) return unit(Math.floor(d / MIN), "minute");
  if (d < DAY) return unit(Math.floor(d / HOUR), "hour");
  if (d < WEEK) return unit(Math.floor(d / DAY), "day");
  if (d < 30 * DAY) return unit(Math.floor(d / WEEK), "week");
  if (d < 365 * DAY) return unit(Math.max(1, Math.floor(d / (30 * DAY))), "month");
  return unit(Math.floor(d / (365 * DAY)), "year");
}

const dateFmt = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric", year: "numeric" });

/** Date added: relative within the last week, otherwise the date. */
export function addedLabel(at: number | null | undefined, now = Date.now() / 1000): string {
  if (!at) return "";
  return now - at < WEEK ? timeAgo(at, now) : dateFmt.format(new Date(at * 1000));
}

/** Last played, always relative. Empty when never played. */
export function playedLabel(at: number | null | undefined, now = Date.now() / 1000): string {
  return at ? timeAgo(at, now) : "";
}

/** Friend activity's short time: "now", "11 min", "2 hr", "1 d". */
export function shortAgo(at: number, now = Date.now() / 1000, live = false): string {
  const d = Math.max(0, now - at);
  if (live || d < MIN) return "now";
  if (d < HOUR) return `${Math.floor(d / MIN)} min`;
  if (d < DAY) return `${Math.floor(d / HOUR)} hr`;
  return `${Math.floor(d / DAY)} d`;
}
