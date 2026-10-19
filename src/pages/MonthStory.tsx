/**
 * This month in words, for the profile page and the phone's You tab: one
 * sentence (how long, and mostly to whom), then a quieter one with the plays,
 * the days and the longest run. Set like the Stats page's hero, in the
 * app's own type; no boxes, no labels in capitals.
 */
import { Skeleton } from "../components/Skeleton";
import type { StatsOverview } from "../lib/statsTypes";
import { monthStory } from "../lib/statsStory";
import "./profile.css";

export function MonthStory({ o, testId }: { o: StatsOverview | undefined; testId?: string }) {
  if (!o)
    return (
      <div className="month-story month-story--loading" aria-hidden="true" data-testid={testId}>
        <Skeleton width="92%" height={24} />
        <Skeleton width="64%" height={24} style={{ marginTop: 8 }} />
        <Skeleton width="48%" height={14} style={{ marginTop: 16 }} />
      </div>
    );
  if (o.totals.plays === 0)
    return (
      <p className="month-story__sentence" data-testid={testId}>
        Nothing played yet this month.
      </p>
    );
  const s = monthStory(o);
  return (
    <div className="month-story" data-testid={testId}>
      <p className="month-story__sentence">
        You listened for <strong>{s.time}</strong> this month
        {s.artist ? (
          <>
            , {s.lead === "mostly to" ? "mostly to" : "more to"} <strong>{s.artist}</strong>
            {s.lead === "more to" && " than anyone else"}.
          </>
        ) : (
          "."
        )}
      </p>
      <p className="month-story__more">
        That's {s.plays} over {s.days}
        {s.streak ? <>, and your longest run was {s.streak}.</> : "."}
      </p>
    </div>
  );
}
