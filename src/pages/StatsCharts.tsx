/**
 * The Stats page's charts, drawn as plain SVG and CSS: minutes per day or
 * month as bars, a year of listening days as a calendar, and listening by
 * hour as a 24-hour clock. Each chart is one keyboard stop (arrow keys move
 * through it and a line above says the numbers) and carries a text summary
 * and table for screen readers.
 */
import { useEffect, useId, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { Skeleton } from "../components/Skeleton";
import { backend } from "../lib/backend";
import { WEEKDAYS, barFractions, formatMinutes, formatPlays, hourLabel, minutes, monthName, shortDate } from "../lib/stats";
import { averageMs, calendarGrid, calendarMonths, calendarSummary, calendarWindow, clockSpokes, clockSummary, seriesSummary, type CalendarCell } from "../lib/statsCharts";
import type { StatsPoint } from "../lib/statsTypes";
import { useLoad } from "../lib/useLoad";

/** Arrow-key movement through a chart's points; returns the new index or null. */
function step(e: KeyboardEvent, i: number | null, n: number, keys: Record<string, number>, wrap = false): number | null {
  if (!n) return null;
  const at = i ?? n - 1;
  let next: number | null = null;
  if (e.key in keys) next = at + keys[e.key];
  if (e.key === "Home") next = 0;
  if (e.key === "End") next = n - 1;
  if (next == null) return null;
  e.preventDefault();
  return wrap ? ((next % n) + n) % n : Math.max(0, Math.min(n - 1, next));
}

const pointLabel = (p: StatsPoint, bucket: "day" | "month") => (bucket === "day" ? shortDate(p.key) : monthName(p.key));
const lastWith = (values: number[]) => {
  for (let i = values.length - 1; i >= 0; i--) if (values[i] > 0) return i;
  return values.length - 1;
};

/** Minutes per day or month as bars, with the average as a line. */
export function BarChart({ points, bucket, label }: { points: StatsPoint[]; bucket: "day" | "month"; label: string }) {
  const id = useId();
  const ms = points.map((p) => p.ms);
  const fr = barFractions(ms);
  const max = Math.max(0, ...ms);
  const avg = averageMs(points);
  const [cur, setCur] = useState<number | null>(null);
  const shown = cur != null ? points[cur] : null;
  const first = points[0];
  const last = points[points.length - 1];
  const mid = points[Math.floor(points.length / 2)];
  const summary = seriesSummary(points, bucket, (p) => pointLabel(p, bucket));
  return (
    <figure className="stats-chart" aria-label={label} aria-describedby={`${id}-sum`}>
      <div className="stats-card__head">
        <h3 className="stats-card__title">Minutes per {bucket}</h3>
        <p className="stats-chart__readout" aria-live="polite" data-testid="bars-readout">
          {shown ? (
            <>
              <strong>{pointLabel(shown, bucket)}</strong> {formatMinutes(shown.ms)} · {formatPlays(shown.plays)}
            </>
          ) : avg > 0 ? (
            `Average ${minutes(avg).toLocaleString()} minutes a ${bucket} you listened`
          ) : (
            "No listening in this range"
          )}
        </p>
      </div>
      <p id={`${id}-sum`} className="sr-only">
        {summary}
      </p>
      <div className="stats-chart__plot">
        {avg > 0 && max > 0 && (
          <span className="stats-chart__avg" style={{ bottom: `${(avg / max) * 100}%` }} aria-hidden="true">
            <span className="stats-chart__avg-label">avg</span>
          </span>
        )}
        <div
          className="stats-chart__bars"
          role="group"
          aria-roledescription="bar chart"
          aria-label={`${label}. Arrow keys move through the ${bucket}s.`}
          tabIndex={0}
          aria-activedescendant={cur != null ? `${id}-b${cur}` : undefined}
          onKeyDown={(e) => {
            const n = step(e, cur, points.length, { ArrowRight: 1, ArrowLeft: -1 });
            if (n != null) setCur(n);
          }}
          onFocus={() => cur == null && setCur(lastWith(ms))}
          onBlur={() => setCur(null)}
          onMouseLeave={() => setCur(null)}
        >
          {points.map((p, i) => (
            <span
              key={p.key}
              id={`${id}-b${i}`}
              role="img"
              aria-label={`${pointLabel(p, bucket)}: ${minutes(p.ms)} minutes, ${formatPlays(p.plays)}`}
              className={`stats-chart__bar ${cur === i ? "is-cur" : ""} ${cur != null && cur !== i ? "is-dim" : ""}`}
              onMouseEnter={() => setCur(i)}
            >
              <span className="stats-chart__fill" style={{ height: `${Math.max(p.ms > 0 ? 2 : 0, fr[i] * 100)}%` }} />
            </span>
          ))}
        </div>
      </div>
      {first && last && (
        <div className="stats-chart__axis" aria-hidden="true">
          <span>{pointLabel(first, bucket)}</span>
          {points.length > 4 && <span>{pointLabel(mid, bucket)}</span>}
          <span>{pointLabel(last, bucket)}</span>
        </div>
      )}
    </figure>
  );
}

/** Listening by hour as a 24-hour clock: midnight at the top, clockwise. */
export function HourClock({ hours, busiest, weekdays, busiestWeekday }: { hours: number[]; busiest: number | null; weekdays: number[]; busiestWeekday: number | null }) {
  const id = useId();
  const size = 220;
  const spokes = useMemo(() => clockSpokes(hours, size, 40, 100), [hours]);
  const [cur, setCur] = useState<number | null>(null);
  const shownHour = cur ?? busiest;
  const wfr = barFractions(weekdays);
  const c = size / 2;
  return (
    <figure className="stats-clock" aria-label="Listening by hour of the day" aria-describedby={`${id}-sum`}>
      <div className="stats-card__head">
        <h3 className="stats-card__title">Your day</h3>
      </div>
      <p id={`${id}-sum`} className="sr-only">
        {clockSummary(hours, busiest)}
      </p>
      <svg
        className="stats-clock__svg"
        viewBox={`-30 -8 ${size + 60} ${size + 16}`}
        role="group"
        aria-roledescription="clock chart"
        aria-label="Hours of the day. Arrow keys move through the hours."
        tabIndex={0}
        aria-activedescendant={cur != null ? `${id}-h${cur}` : undefined}
        onKeyDown={(e) => {
          // Round the clock: past 11 PM comes midnight.
          const n = step(e, cur ?? busiest, 24, { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 }, true);
          if (n != null) setCur(n);
        }}
        onBlur={() => setCur(null)}
        onMouseLeave={() => setCur(null)}
      >
        <circle cx={c} cy={c} r={100} className="stats-clock__ring" />
        <circle cx={c} cy={c} r={70} className="stats-clock__ring stats-clock__ring--mid" />
        {spokes.map((s) => (
          <g key={s.hour} id={`${id}-h${s.hour}`} role="img" aria-label={`${hourLabel(s.hour)}: ${minutes(hours[s.hour])} minutes`} onMouseEnter={() => setCur(s.hour)}>
            <line
              x1={s.x1}
              y1={s.y1}
              x2={s.x2}
              y2={s.y2}
              className={`stats-clock__spoke ${shownHour === s.hour ? "is-cur" : ""}`}
              style={{ opacity: hours[s.hour] > 0 ? 0.4 + s.fraction * 0.6 : 0.25 }}
            />
            <line x1={s.x1} y1={s.y1} x2={c + (s.x1 - c) * 2.6} y2={c + (s.y1 - c) * 2.6} className="stats-clock__hit" />
          </g>
        ))}
        <text x={c} y={-1} className="stats-clock__tick" textAnchor="middle">
          12 AM
        </text>
        <text x={size + 4} y={c + 4} className="stats-clock__tick" textAnchor="start">
          6 AM
        </text>
        <text x={c} y={size + 7} className="stats-clock__tick" textAnchor="middle">
          12 PM
        </text>
        <text x={-4} y={c + 4} className="stats-clock__tick" textAnchor="end">
          6 PM
        </text>
        <text x={c} y={c - 2} className="stats-clock__hour" textAnchor="middle" aria-hidden="true">
          {shownHour != null ? hourLabel(shownHour) : "–"}
        </text>
        <text x={c} y={c + 16} className="stats-clock__caption" textAnchor="middle" aria-hidden="true">
          {cur != null ? `${minutes(hours[cur])} min` : shownHour != null ? "busiest" : "no plays"}
        </text>
      </svg>
      <div className="stats-week" aria-label={`Busiest weekday: ${busiestWeekday != null ? WEEKDAYS[busiestWeekday] : "none yet"}`} role="img">
        {WEEKDAYS.map((d, i) => (
          <span key={d} className={`stats-week__day ${busiestWeekday === i ? "is-top" : ""}`}>
            <span className="stats-week__bar">
              <span style={{ height: `${Math.max(weekdays[i] > 0 ? 8 : 0, wfr[i] * 100)}%` }} />
            </span>
            <span className="stats-week__label" aria-hidden="true">
              {d.slice(0, 1)}
            </span>
          </span>
        ))}
      </div>
    </figure>
  );
}

const CELL = 12;
const GAP = 3;
const LEFT = 30;
const TOP = 18;
const DOW = [
  [1, "Mon"],
  [3, "Wed"],
  [5, "Fri"],
] as const;

function cellLabel(c: CalendarCell): string {
  const d = new Date(`${c.day}T12:00:00`);
  const wd = WEEKDAYS[d.getDay()].slice(0, 3);
  return `${wd}, ${shortDate(c.day)}: ${c.ms > 0 ? `${formatMinutes(c.ms)}, ${formatPlays(c.plays)}` : "no listening"}${c.inRange ? "" : " (outside this range)"}`;
}

/** A year of listening days, one square a day, darker for more listening. */
export function ListeningCalendar({ from, to }: { from: string; to: string }) {
  const id = useId();
  const w = useMemo(() => calendarWindow(from, to), [from, to]);
  const { data: points, error } = useLoad(() => backend.statsDays(w.from, w.to), [w.from, w.to]);
  const grid = useMemo(() => (points ? calendarGrid(w, points, { from, to }) : null), [w, points, from, to]);
  const cells = useMemo(() => grid?.weeks.flat() ?? [], [grid]);
  const [cur, setCur] = useState<number | null>(null);
  const shown = cur != null ? cells[cur] : null;
  // On a narrow window the calendar scrolls sideways; start at its latest weeks.
  const scroll = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const el = scroll.current;
    if (el) el.scrollLeft = el.scrollWidth;
  }, [grid]);
  const width = LEFT + (grid?.weeks.length ?? 53) * (CELL + GAP);
  const height = TOP + 7 * (CELL + GAP);
  const focusStart = () => {
    const i = cells.findIndex((c) => c.day === grid?.busiest?.day);
    return i >= 0
      ? i
      : Math.max(
          0,
          cells.findIndex((c) => c.day === to),
        );
  };
  return (
    <figure className="stats-cal" aria-label="Listening calendar" aria-describedby={`${id}-sum`} data-testid="stats-calendar">
      <div className="stats-card__head">
        <h3 className="stats-card__title">
          Listening days <span className="stats-card__sub">{w.title}</span>
        </h3>
        <p className="stats-chart__readout" aria-live="polite">
          {shown ? cellLabel(shown) : grid ? `${grid.daysListened.toLocaleString()} ${grid.daysListened === 1 ? "day" : "days"} with listening` : ""}
        </p>
      </div>
      {error ? (
        <p className="stats-muted">Couldn't load the calendar.</p>
      ) : !grid ? (
        <Skeleton width="100%" height={120} radius="card" />
      ) : (
        <>
          <p id={`${id}-sum`} className="sr-only">
            {calendarSummary(w, grid)}
          </p>
          <table className="sr-only">
            <caption>Listening by month</caption>
            <thead>
              <tr>
                <th scope="col">Month</th>
                <th scope="col">Days with listening</th>
                <th scope="col">Minutes</th>
              </tr>
            </thead>
            <tbody>
              {calendarMonths(grid).map((m) => (
                <tr key={m.month}>
                  <th scope="row">{m.month}</th>
                  <td>{m.days}</td>
                  <td>{m.minutes.toLocaleString()}</td>
                </tr>
              ))}
            </tbody>
          </table>
          <div className="stats-cal__scroll" ref={scroll}>
            <svg
              className="stats-cal__svg"
              viewBox={`0 0 ${width} ${height}`}
              role="group"
              aria-roledescription="calendar chart"
              aria-label="Days. Arrow keys move by day and week."
              tabIndex={0}
              aria-activedescendant={cur != null ? `${id}-d${cur}` : undefined}
              onKeyDown={(e) => {
                let n = step(e, cur ?? focusStart(), cells.length, { ArrowDown: 1, ArrowUp: -1, ArrowRight: 7, ArrowLeft: -7 });
                // A year's padding days aren't drawn: step back inside it.
                const back = e.key === "ArrowLeft" || e.key === "ArrowUp" || e.key === "Home" ? 1 : -1;
                while (n != null && !cells[n].inWindow && cells[n + back]) n += back;
                if (n != null) setCur(n);
              }}
              onFocus={() => cur == null && setCur(focusStart())}
              onBlur={() => setCur(null)}
              onMouseLeave={() => setCur(null)}
            >
              {grid.months.map((m) => (
                <text key={`${m.label}${m.week}`} x={LEFT + m.week * (CELL + GAP)} y={11} className="stats-cal__label">
                  {m.label}
                </text>
              ))}
              {DOW.map(([row, name]) => (
                <text key={name} x={LEFT - 6} y={TOP + row * (CELL + GAP) + CELL - 2} className="stats-cal__label" textAnchor="end">
                  {name}
                </text>
              ))}
              {grid.weeks.map((week, wi) =>
                week.map((c, di) => {
                  if (!c.inWindow) return null;
                  const i = wi * 7 + di;
                  return (
                    <rect
                      key={c.day}
                      id={`${id}-d${i}`}
                      role="img"
                      aria-label={cellLabel(c)}
                      x={LEFT + wi * (CELL + GAP)}
                      y={TOP + di * (CELL + GAP)}
                      width={CELL}
                      height={CELL}
                      rx={2.5}
                      className={`stats-cal__day stats-cal__day--${c.level} ${c.after ? "is-after" : c.inRange ? "" : "is-out"} ${cur === i ? "is-cur" : ""}`}
                      data-level={c.level}
                      onMouseEnter={() => setCur(i)}
                    />
                  );
                }),
              )}
            </svg>
          </div>
          <div className="stats-cal__legend" aria-hidden="true">
            <span>Less</span>
            {[0, 1, 2, 3, 4].map((l) => (
              <span key={l} className={`stats-cal__swatch stats-cal__day--${l}`} />
            ))}
            <span>More</span>
          </div>
        </>
      )}
    </figure>
  );
}
