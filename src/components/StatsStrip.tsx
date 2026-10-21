/**
 * Your numbers for one song, artist or album: plays, minutes, rank in your
 * all-time top, first and last listen, and a sparkline of plays per month.
 * Only in the "Your stats" dialog, opened from a menu: everyday pages carry
 * no charts.
 */
import { useEffect, useState } from "react";
import { backend } from "../lib/backend";
import { formatMinutes, formatPlays, monthName, playedFromLabel, sparkPath } from "../lib/stats";
import type { ItemStats, StatsItemKind } from "../lib/statsTypes";
import { timeAgo } from "../lib/relativeTime";
import "./stats-strip.css";

const KIND_TOP: Record<StatsItemKind, string> = { song: "songs", artist: "artists", album: "albums" };
const dateFmt = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric", year: "numeric" });

/** The sparkline: plays per month, last 12 months, with a dot on this month. */
export function Sparkline({ values, label, width = 120, height = 32 }: { values: number[]; label: string; width?: number; height?: number }) {
  const d = sparkPath(values, width, height);
  const end = d.split(" L").pop()?.replace(/^M/, "").split(" ").map(Number) ?? [];
  return (
    <svg className="sparkline" width={width} height={height} viewBox={`0 0 ${width} ${height}`} role="img" aria-label={label}>
      <line x1={2} x2={width - 2} y1={height - 2} y2={height - 2} className="sparkline__base" />
      <path d={`${d} L${width - 2} ${height - 2} L2 ${height - 2} Z`} className="sparkline__fill" />
      <path d={d} className="sparkline__line" />
      {end.length === 2 && <circle cx={end[0]} cy={end[1]} r={2.5} className="sparkline__dot" />}
    </svg>
  );
}

/** The month with the most plays of the last 12, or null. */
export function peakMonth(s: ItemStats): string | null {
  const best = s.series.reduce<ItemStats["series"][number] | null>((a, p) => (p.plays > 0 && (!a || p.plays >= a.plays) ? p : a), null);
  return best ? monthName(best.key) : null;
}

/** What the sparkline says to a screen reader. */
export function sparkLabel(s: ItemStats): string {
  const parts = s.series.map((p) => `${monthName(p.key)}: ${p.plays}`);
  return `Plays per month, last 12 months. ${parts.join(", ")}`;
}

export function StatsStripView({ stats, kind, sparkWidth = 120 }: { stats: ItemStats; kind: StatsItemKind; sparkWidth?: number }) {
  const first = stats.first;
  const from = first ? playedFromLabel(first.from) : null;
  const peak = peakMonth(stats);
  return (
    <div className="stats-strip__body">
      <dl className="stats-strip__nums">
        <div className="stats-strip__num">
          <dt>Plays</dt>
          <dd>{formatPlays(stats.plays)}</dd>
        </div>
        <div className="stats-strip__num">
          <dt>Listened</dt>
          <dd>{formatMinutes(stats.ms)}</dd>
        </div>
        {stats.rank != null && (
          <div className="stats-strip__num">
            <dt>All-time rank</dt>
            <dd>
              <span className={`stats-strip__rank ${stats.rank <= 10 ? "is-top" : ""}`}>#{stats.rank.toLocaleString()}</span> <span className="stats-strip__muted">in your top {KIND_TOP[kind]}</span>
            </dd>
          </div>
        )}
        {first && (
          <div className="stats-strip__num">
            <dt>First listen</dt>
            <dd>
              {dateFmt.format(new Date(first.at))}
              <span className="stats-strip__muted">
                {kind === "song" ? "" : ` · ${first.song}`}
                {from ? ` ${from}` : ""}
              </span>
            </dd>
          </div>
        )}
        {stats.lastAt != null && (
          <div className="stats-strip__num">
            <dt>Last listen</dt>
            <dd>{timeAgo(Math.floor(stats.lastAt / 1000))}</dd>
          </div>
        )}
      </dl>
      <div className="stats-strip__spark">
        <Sparkline values={stats.series.map((p) => p.plays)} label={sparkLabel(stats)} width={sparkWidth} height={sparkWidth > 200 ? 48 : 32} />
        <span className="stats-strip__spark-label" aria-hidden="true">
          {peak ? `Peak: ${peak.split(" ")[0].slice(0, 3)} ${peak.split(" ")[1]}` : "Last 12 months"}
        </span>
      </div>
    </div>
  );
}

/** Load one item's stats. */
export function useItemStats(kind: StatsItemKind, id: string, on = true): ItemStats | null | "error" {
  const [stats, setStats] = useState<ItemStats | null | "error">(null);
  useEffect(() => {
    if (!on) return;
    let live = true;
    setStats(null);
    backend.statsItem(kind, id).then(
      (s) => live && setStats(s),
      () => live && setStats("error"),
    );
    return () => {
      live = false;
    };
  }, [kind, id, on]);
  return stats;
}
