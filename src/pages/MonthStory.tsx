/**
 * This month as one plain line, for the profile page and the phone's You
 * tab: how long, and mostly to whom ("14 hrs this month, mostly Copperline
 * Choir."). No big sentence, no boxes, no labels in capitals.
 */
import { Skeleton } from "../components/Skeleton";
import { duration } from "../lib/statsStory";
import type { StatsOverview } from "../lib/statsTypes";
/** "40 min", "1 hr 40 min", "14 hrs" (minutes drop off at ten hours). */

/** "14 hrs", "1 hr", "40 min". */
export function shortListen(ms: number): string {
  return duration(ms);
}

/** The month's line: its time, and the artist who took a quarter of it or more ("mostly"), else the one played most. */
export function monthLine(o: StatsOverview): string {
  if (o.totals.plays === 0) return "Nothing played yet this month.";
  const a = o.topArtists[0];
  const time = `${shortListen(o.totals.ms)} this month`;
  if (!a) return `${time}.`;
  const share = o.totals.ms > 0 ? a.ms / o.totals.ms : 0;
  return share >= 0.25 ? `${time}, mostly ${a.name}.` : `${time}, ${a.name} most of all.`;
}

export function MonthStory({ o, testId }: { o: StatsOverview | undefined; testId?: string }) {
  if (!o)
    return (
      <div className="month-story month-story--loading" aria-hidden="true" data-testid={testId}>
        <Skeleton width="56%" height={16} />
      </div>
    );
  return (
    <p className="month-story" data-testid={testId}>
      {monthLine(o)}
    </p>
  );
}
