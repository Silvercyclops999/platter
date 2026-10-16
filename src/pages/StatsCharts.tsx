/**
 * The Stats page's "When you listened": three short stories, each a plain
 * sentence written from the numbers with a small chart under it. Listening
 * days as month calendars from the first play, minutes per day or month as
 * bars on a light scale with the best one marked, and your day as 24 hourly
 * bars with the peak marked, beside the weekdays. Charts are plain HTML;
 * each is one keyboard stop (arrow keys move, the line under it says the
 * numbers) with a summary and a table for screen readers.
 */
import { useId, useMemo, useState, type KeyboardEvent, type ReactNode } from "react";
import { Art } from "../components/Art";
import { Skeleton } from "../components/Skeleton";
import { backend } from "../lib/backend";
import { WEEKDAYS, addDays, formatPlays, hourLabel, minutes, monthBounds, monthName, shortDate } from "../lib/stats";
import { seriesSummary } from "../lib/statsCharts";
import {
  activeSlice,
  axisTicks,
  bestPoint,
  calendarDayLabel,
  calendarMonths,
  activeWeeks,
  dayLabel,
  daySentence,
  daysSentence,
  duration,
  hourOrder,
  isMostlyEmpty,
  minuteScale,
  minutesFootnote,
  minutesSentence,
  peakHours,
  standoutWeekday,
  tickLabel,
  type MinuteScale,
  type MonthCell,
} from "../lib/statsStory";
import type { StatsOverview, StatsPoint } from "../lib/statsTypes";
import { useLoad } from "../lib/useLoad";

/** Arrow-key movement through a chart's points; returns the new index or null. */
function step(e: KeyboardEvent, i: number | null, n: number, keys: Record<string, number>): number | null {
  if (!n) return null;
  const at = i ?? n - 1;
  let next: number | null = null;
  if (e.key in keys) next = at + keys[e.key];
  if (e.key === "Home") next = 0;
  if (e.key === "End") next = n - 1;
  if (next == null) return null;
  e.preventDefault();
  return Math.max(0, Math.min(n - 1, next));
}

const pointLabel = (p: StatsPoint, bucket: "day" | "month") => (bucket === "day" ? shortDate(p.key) : monthName(p.key));
const axisLabel = (p: StatsPoint, bucket: "day" | "month", year: string) => {
  if (bucket === "day") return dayLabel(p.key, year).replace(/, \d{4}$/, "");
  const [y, m] = p.key.split("-");
  const short = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"][Number(m) - 1];
  return m === "01" ? `${short} ${y}` : short;
};

interface BarItem {
  key: string;
  ms: number;
  /** What the bar says to a screen reader. */
  label: string;
}

/**
 * Bars on a shared baseline. Highlighted bars take the accent; the rest
 * stay neutral. With a scale, hairlines mark round minutes.
 */
function Bars({
  items,
  highlight,
  scale,
  ticks,
  note,
  groupLabel,
  cur,
  setCur,
  startAt,
  variant,
}: {
  items: BarItem[];
  highlight: Set<number>;
  scale?: MinuteScale;
  ticks: { i: number; text: string; align?: "start" | "center" }[];
  note?: { i: number; content: ReactNode } | null;
  groupLabel: string;
  cur: number | null;
  setCur: (i: number | null) => void;
  startAt: () => number;
  variant: "minutes" | "hours" | "week";
}) {
  const id = useId();
  const n = items.length;
  const topMs = scale ? scale.top * 60_000 : Math.max(0, ...items.map((b) => b.ms));
  const pct = (ms: number) => (topMs > 0 ? Math.min(100, (ms / topMs) * 100) : 0);
  const noteAlign = note ? ((note.i + 0.5) / n < 0.15 ? "start" : (note.i + 0.5) / n > 0.85 ? "end" : "center") : "center";
  return (
    <div className={`bars bars--${variant} ${scale ? "has-scale" : ""}`}>
      <div className="bars__area">
        {scale?.ticks.map((t) => (
          <span key={t} className="bars__grid" style={{ bottom: `${(t / scale.top) * 100}%` }} aria-hidden="true">
            <span className="bars__tick">{tickLabel(t)}</span>
          </span>
        ))}
        <div
          className={`bars__row ${n > 40 ? "is-dense" : ""}`}
          role="group"
          aria-label={groupLabel}
          tabIndex={0}
          aria-activedescendant={cur != null ? `${id}-b${cur}` : undefined}
          onKeyDown={(e) => {
            const next = step(e, cur ?? startAt(), n, { ArrowRight: 1, ArrowLeft: -1 });
            if (next != null) setCur(next);
          }}
          onFocus={() => cur == null && setCur(startAt())}
          onBlur={() => setCur(null)}
          onMouseLeave={() => setCur(null)}
        >
          {items.map((b, i) => (
            <span key={b.key} id={`${id}-b${i}`} role="img" aria-label={b.label} className={`bars__bar ${highlight.has(i) ? "is-peak" : ""} ${cur === i ? "is-cur" : ""}`} onMouseEnter={() => setCur(i)}>
              <span className="bars__fill" style={{ height: `${b.ms > 0 ? Math.max(1.5, pct(b.ms)) : 0}%` }} />
            </span>
          ))}
        </div>
        {note && (
          <span className={`bars__note bars__note--${noteAlign}`} style={{ left: `${((note.i + 0.5) / n) * 100}%`, bottom: `${pct(items[note.i].ms)}%` }} aria-hidden="true">
            {note.content}
          </span>
        )}
      </div>
      <div className="bars__x" aria-hidden="true">
        {ticks.map((t) => (
          <span key={t.i} className={`bars__xlabel bars__xlabel--${t.align ?? "center"}`} style={{ left: `${((t.align === "start" ? t.i : t.i + 0.5) / n) * 100}%` }}>
            {t.text}
          </span>
        ))}
      </div>
    </div>
  );
}

/**
 * Minutes per day or month as bars on a light minutes scale, the best one
 * in the accent with its value (and its top album's cover, when given).
 */
export function BarChart({
  points,
  bucket,
  label,
  best,
  footnote,
}: {
  points: StatsPoint[];
  bucket: "day" | "month";
  label: string;
  best?: { thumb: string | null } | null;
  footnote?: string;
}) {
  const id = useId();
  const [cur, setCur] = useState<number | null>(null);
  const top = bestPoint(points);
  const topIndex = top ? points.indexOf(top) : -1;
  const scale = minuteScale(top?.ms ?? 0);
  const year = points[points.length - 1]?.key.slice(0, 4) ?? "";
  const shown = cur != null ? points[cur] : null;
  const lastWith = () => {
    for (let i = points.length - 1; i >= 0; i--) if (points[i].ms > 0) return i;
    return points.length - 1;
  };
  return (
    <figure className="chart" aria-label={label} aria-describedby={`${id}-sum`}>
      <p id={`${id}-sum`} className="sr-only">
        {seriesSummary(points, bucket, (p) => pointLabel(p, bucket))}
      </p>
      <Bars
        variant="minutes"
        items={points.map((p) => ({ key: p.key, ms: p.ms, label: `${pointLabel(p, bucket)}: ${minutes(p.ms)} minutes, ${formatPlays(p.plays)}` }))}
        highlight={new Set(topIndex >= 0 ? [topIndex] : [])}
        scale={scale}
        ticks={axisTicks(points, bucket).map((i) => ({ i, text: axisLabel(points[i], bucket, year) }))}
        note={
          top
            ? {
                i: topIndex,
                content: (
                  <>
                    {best?.thumb && (
                      <span className="bars__cover">
                        <Art path={best.thumb} size={28} />
                      </span>
                    )}
                    <span className="bars__value">{duration(top.ms)}</span>
                  </>
                ),
              }
            : null
        }
        groupLabel={`${label}. Arrow keys move through the ${bucket}s.`}
        cur={cur}
        setCur={setCur}
        startAt={lastWith}
      />
      <p className="chart__foot" aria-live="polite" data-testid="bars-readout">
        {shown ? (
          <>
            <strong>{pointLabel(shown, bucket)}</strong> {duration(shown.ms)} · {formatPlays(shown.plays)}
          </>
        ) : (
          (footnote ?? "")
        )}
      </p>
    </figure>
  );
}

/** Listening by hour (24 bars, the peak hours in the accent) and by weekday. */
export function DayCharts({ hours, weekdays }: { hours: number[]; weekdays: number[] }) {
  const id = useId();
  const [pos, setPos] = useState<number | null>(null);
  const [wd, setWd] = useState<number | null>(null);
  const peak = peakHours(hours);
  // A late-night peak stays in one piece: the hours start at 6 AM.
  const order = hourOrder(peak);
  const peakSet = new Set(peak ? peak.hours.filter((h) => hours[h] > 0).map((h) => order.indexOf(h)) : []);
  const hour = pos != null ? order[pos] : null;
  const top = standoutWeekday(weekdays);
  const total = hours.reduce((s, h) => s + h, 0);
  const foot =
    hour != null ? (
      <>
        <strong>
          {hourLabel(hour)} to {hourLabel((hour + 1) % 24)}
        </strong>{" "}
        {duration(hours[hour])}
      </>
    ) : wd != null ? (
      <>
        <strong>{WEEKDAYS[wd]}s</strong> {duration(weekdays[wd])}
      </>
    ) : peak ? (
      `${Math.round(peak.share * 100)}% of your listening falls between ${hourLabel(peak.start)} and ${hourLabel((peak.start + peak.hours.length) % 24)}.`
    ) : (
      ""
    );
  return (
    <div className="daycharts">
      <figure className="chart chart--hours" aria-label="Listening by hour of the day" aria-describedby={`${id}-h`}>
        <table id={`${id}-h`} className="sr-only">
          <caption>Minutes listened by hour of the day</caption>
          <tbody>
            {hours.map((ms, h) => (
              <tr key={h}>
                <th scope="row">{hourLabel(h)}</th>
                <td>{minutes(ms)}</td>
              </tr>
            ))}
          </tbody>
        </table>
        <Bars
          variant="hours"
          items={order.map((h) => ({ key: String(h), ms: hours[h], label: `${hourLabel(h)}: ${minutes(hours[h])} minutes` }))}
          highlight={peakSet}
          ticks={[0, 6, 12, 18].map((h) => ({ i: order.indexOf(h), text: hourLabel(h), align: "start" as const }))}
          groupLabel="Hours of the day. Arrow keys move through the hours."
          cur={pos}
          setCur={setPos}
          startAt={() => (peak ? order.indexOf(peak.start) : 0)}
        />
      </figure>
      <figure className="chart chart--week" aria-label="Listening by day of the week">
        <Bars
          variant="week"
          items={weekdays.map((ms, d) => ({ key: WEEKDAYS[d], ms, label: `${WEEKDAYS[d]}: ${minutes(ms)} minutes` }))}
          highlight={new Set(top != null ? [top] : [])}
          ticks={WEEKDAYS.map((d, i) => ({ i, text: d.slice(0, 3) }))}
          groupLabel="Days of the week. Arrow keys move through the days."
          cur={wd}
          setCur={setWd}
          startAt={() => top ?? 0}
        />
      </figure>
      <p className="chart__foot daycharts__foot" aria-live="polite" data-testid="day-readout">
        {total > 0 ? foot : ""}
      </p>
    </div>
  );
}

const DOW = ["S", "M", "T", "W", "T", "F", "S"];

function DowRow() {
  return (
    <>
      {DOW.map((d, i) => (
        <span key={i} className="cal__dow" aria-hidden="true">
          {d}
        </span>
      ))}
    </>
  );
}

/**
 * Listening days as calendars from the first play in the range: whole
 * months, or for a mostly empty range only the weeks you listened in, with
 * the quiet weeks between them counted. The best day takes the accent.
 */
export function ListeningCalendar({ points, from, to, footnote }: { points: StatsPoint[]; from: string; to: string; footnote: string }) {
  const id = useId();
  const sparse = useMemo(() => isMostlyEmpty(points.filter((p) => p.key >= from), to), [points, from, to]);
  const months = useMemo(() => (sparse ? null : calendarMonths(points, from, to)), [sparse, points, from, to]);
  const weeks = useMemo(() => (sparse ? activeWeeks(points, from, to) : null), [sparse, points, from, to]);
  const best = months?.best ?? weeks?.best ?? null;
  const cells = useMemo(
    () => (months ? months.months.flatMap((m) => m.weeks.flat()) : (weeks?.weeks.flatMap((w) => w.cells) ?? [])).filter((c): c is MonthCell => !!c && c.inRange),
    [months, weeks],
  );
  const [cur, setCur] = useState<number | null>(null);
  const size = weeks ? "md" : months!.months.length <= 2 ? "lg" : months!.months.length <= 4 ? "md" : "sm";
  const shown = cur != null ? cells[cur] : null;
  const startAt = () => {
    const i = cells.findIndex((c) => c.day === best?.day);
    return i >= 0 ? i : cells.length - 1;
  };
  const day = (c: MonthCell | null, key: string) => {
    if (!c) return <span key={key} className="cal__pad" />;
    const k = c.inRange ? cells.indexOf(c) : -1;
    return (
      <span
        key={c.day}
        id={k >= 0 ? `${id}-d${k}` : undefined}
        role="img"
        aria-label={calendarDayLabel(c)}
        className={`cal__day cal__day--${c.level} ${c.inRange ? "" : "is-out"} ${best?.day === c.day ? "is-best" : ""} ${k >= 0 && cur === k ? "is-cur" : ""}`}
        onMouseEnter={() => setCur(k >= 0 ? k : null)}
      >
        {c.date}
      </span>
    );
  };
  const rows = months
    ? months.months.map((m) => ({ key: m.month, label: monthName(m.month), days: m.days, ms: m.ms }))
    : (weeks?.weeks ?? []).map((w) => {
        const on = w.cells.filter((c) => c.ms > 0);
        return { key: w.start, label: `Week of ${shortDate(w.start)}`, days: on.length, ms: on.reduce((s, c) => s + c.ms, 0) };
      });
  return (
    <figure className={`cal cal--${size} ${weeks ? "cal--weeks" : ""}`} aria-label="Listening days" data-testid="stats-calendar">
      <table className="sr-only">
        <caption>{months ? "Listening by month" : "Weeks with listening"}</caption>
        <thead>
          <tr>
            <th scope="col">{months ? "Month" : "Week"}</th>
            <th scope="col">Days with listening</th>
            <th scope="col">Minutes</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => (
            <tr key={r.key}>
              <th scope="row">{r.label}</th>
              <td>{r.days}</td>
              <td>{minutes(r.ms).toLocaleString()}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <div
        className={`cal__months ${weeks && weeks.weeks.length > 6 ? "is-split" : ""}`}
        role="group"
        aria-label="Days. Arrow keys move by day and week."
        tabIndex={0}
        aria-activedescendant={cur != null ? `${id}-d${cur}` : undefined}
        onKeyDown={(e) => {
          const n = step(e, cur ?? startAt(), cells.length, { ArrowRight: 1, ArrowLeft: -1, ArrowDown: 7, ArrowUp: -7 });
          if (n != null) setCur(n);
        }}
        onFocus={() => cur == null && setCur(startAt())}
        onBlur={() => setCur(null)}
        onMouseLeave={() => setCur(null)}
      >
        {months?.months.map((m) => (
          <div key={m.month} className="cal__month">
            <span className="cal__label">{m.label}</span>
            <span className="cal__grid">
              <DowRow />
              {m.weeks.flat().map((c, i) => day(c, `p${i}`))}
            </span>
          </div>
        ))}
        {weeks && (
          <div className="cal__weeks">
            <span className="cal__week cal__week--head" aria-hidden="true">
              <span />
              <DowRow />
            </span>
            {weeks.weeks.map((w) => (
              <span key={w.start} className="cal__week-wrap">
                {w.quietBefore > 0 && <span className="cal__quiet">{w.quietBefore === 1 ? "1 quiet week" : `${w.quietBefore} quiet weeks`}</span>}
                <span className="cal__week">
                  <span className="cal__week-label">{dayLabel(w.start, to.slice(0, 4))}</span>
                  {w.cells.map((c) => day(c, c.day))}
                </span>
              </span>
            ))}
          </div>
        )}
      </div>
      <p className="chart__foot" aria-live="polite" data-testid="cal-readout">
        {shown ? calendarDayLabel(shown) : weeks ? `Only the weeks you listened in. ${footnote}` : footnote}
      </p>
    </figure>
  );
}

/** One story: a small heading, the sentence, the chart under it. */
function Story({ id, heading, sentence, layout = "stack", children }: { id: string; heading: string; sentence: ReactNode; layout?: "stack" | "side"; children: ReactNode }) {
  return (
    <article className={`story story--${layout}`} aria-labelledby={id} data-testid={id}>
      <div className="story__text">
        <h3 id={id} className="story__heading">
          {heading}
        </h3>
        <p className="story__sentence">{sentence}</p>
      </div>
      <div className="story__chart">{children}</div>
    </article>
  );
}

/** Daily points for the calendar: the overview's own when it's by day, else the range's latest year. */
function useDaily(o: StatsOverview, end: string): { points: StatsPoint[] | undefined; from: string; error: boolean } {
  const from = o.bucket === "day" ? o.from : o.from > addDays(end, -364) ? o.from : addDays(end, -364);
  const { data, error } = useLoad(() => (o.bucket === "day" ? Promise.resolve(o.series) : backend.statsDays(from, end)), [o, from, end]);
  return { points: data, from, error: !!error };
}

/** "When you listened": listening days, minutes per day or month, and your day, each led by a sentence. */
export function WhenYouListened({ o, title, today }: { o: StatsOverview; title: string; today: string }) {
  // A range can end after today (this whole year); stories stop at today.
  const end = o.to < today ? o.to : today;
  const year = end.slice(0, 4);
  const daily = useDaily(o, end);
  const series = useMemo(() => o.series.filter((p) => p.key <= (o.bucket === "day" ? end : end.slice(0, 7))), [o.series, o.bucket, end]);
  const slice = useMemo(() => activeSlice(series), [series]);
  const best = bestPoint(slice.points);
  const bestRange = best ? (o.bucket === "day" ? { from: best.key, to: best.key } : monthBounds(best.key)) : null;
  const top = useLoad(() => (bestRange ? backend.statsOverview(bestRange, 1) : Promise.resolve(null)), [best?.key, o.bucket]);
  const album = top.data?.topAlbums[0];
  const albumInfo = album && top.data && top.data.totals.ms > 0 ? { title: album.title, share: album.ms / top.data.totals.ms } : null;
  const empty = o.totals.plays === 0;
  const croppedNote = slice.croppedFrom ? `From your first play here, ${o.bucket === "day" ? dayLabel(slice.croppedFrom, year) : monthName(slice.croppedFrom)}. ` : "";
  const firstOn = daily.points?.find((p) => p.ms > 0 && p.key >= daily.from);
  const calNote = [
    daily.from > o.from ? "The last 12 months." : firstOn && firstOn.key > o.from ? `Nothing before ${dayLabel(firstOn.key, year)}.` : "",
    `${o.totals.days.toLocaleString()} ${o.totals.days === 1 ? "day" : "days"} with listening${daily.from > o.from ? " in all" : ""}.`,
  ]
    .filter(Boolean)
    .join(" ");
  return (
    <section className="stats-section stats-when" aria-labelledby="stats-charts" data-shot="charts">
      <h2 id="stats-charts" className="stats-section__title">
        When you listened
      </h2>
      {empty ? (
        <p className="story__sentence">Nothing played in this range yet.</p>
      ) : (
        <>
          <Story id="story-days" heading="Listening days" layout="side" sentence={daily.points ? daysSentence({ from: o.from, to: end, days: o.totals.days, points: daily.points }) : <Skeleton width="80%" height={28} />}>
            {daily.error ? (
              <p className="stats-muted">Couldn't load the calendar.</p>
            ) : !daily.points ? (
              <Skeleton width="100%" height={160} radius="card" />
            ) : (
              <ListeningCalendar points={daily.points} from={daily.from} to={end} footnote={calNote} />
            )}
          </Story>
          <Story id="story-minutes" heading={`Minutes per ${o.bucket}`} sentence={minutesSentence(slice.points, o.bucket, year, albumInfo)}>
            <BarChart points={slice.points} bucket={o.bucket} label={`Minutes listened per ${o.bucket}, ${title}`} best={album ? { thumb: album.thumb } : null} footnote={`${croppedNote}${minutesFootnote(slice.points, o.bucket)}`} />
          </Story>
          <Story id="story-day" heading="Your day" sentence={daySentence(o.hours, o.weekdays, o.totals.days)}>
            <DayCharts hours={o.hours} weekdays={o.weekdays} />
          </Story>
        </>
      )}
    </section>
  );
}
