/**
 * Listening time in words, for the Stats lists and the mixes. Pure, so
 * tests check it.
 */
import { minutes } from "./stats";

/** "45 min", "1 hr", "2 hrs 10 min", "38 hrs" (minutes drop off at ten hours). */
export function duration(ms: number): string {
  const m = minutes(ms);
  if (m < 60) return `${m} min`;
  const h = Math.floor(m / 60);
  const rest = m % 60;
  const hrs = `${h.toLocaleString()} ${h === 1 ? "hr" : "hrs"}`;
  return h >= 10 || rest === 0 ? hrs : `${hrs} ${rest} min`;
}
