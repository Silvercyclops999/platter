/**
 * Your numbers for one song, artist or album: plays, minutes, rank in your
 * all-time top, first and last listen, and a sparkline of plays per month.
 * Shows on artist and album pages under the action row (nothing when you
 * haven't played it, or the strip is off in Settings), and in the "Your
 * stats" dialog for any of the three.
 */
import { useEffect, useState } from "react";
import { backend } from "../lib/backend";
import { formatMinutes, formatPlays, monthName, playedFromLabel, sparkPath } from "../lib/stats";
import type { ItemStats, StatsItemKind } from "../lib/statsTypes";
import { timeAgo } from "../lib/relativeTime";
import { useSettings } from "../stores/settings";
import "./stats-strip.css";

const KIND_TOP: Record<StatsItemKind, string> = { song: "songs", artist: "artists", album: "albums" };
const dateFmt = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric", year: "numeric" });

/** The sparkline: plays per month, last 12 months. */
export function Sparkline({ values, label, width = 120, height = 32 }: { values: number[]; label: string; width?: number; height?: number }) {
  const d = sparkPath(values, width, height);
  return (
    <svg className="sparkline" width={width} height={height} viewBox={`0 0 ${width} ${height}`} role="img" aria-label={label}>
      <path d={`${d} L${width - 2} ${height - 2} L2 ${height - 2} Z`} className="sparkline__fill" />
      <path d={d} className="sparkline__line" />
    </svg>
  );
}

/** What the sparkline says to a screen reader. */
export function sparkLabel(s: ItemStats): string {
  const parts = s.series.map((p) => `${monthName(p.key)}: ${p.plays}`);
  return `Plays per month, last 12 months. ${parts.join(", ")}`;
}

export function StatsStripView({ stats, kind, sparkWidth = 120 }: { stats: ItemStats; kind: StatsItemKind; sparkWidth?: number }) {
  const first = stats.first;
  const from = first ? playedFromLabel(first.from) : null;
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
              #{stats.rank.toLocaleString()} <span className="stats-strip__muted">in your top {KIND_TOP[kind]}</span>
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
      <Sparkline values={stats.series.map((p) => p.plays)} label={sparkLabel(stats)} width={sparkWidth} height={sparkWidth > 200 ? 48 : 32} />
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

/** The strip on an artist or album page. */
export function StatsStrip({ kind, id }: { kind: StatsItemKind; id: string }) {
  const on = useSettings((s) => s.settings?.statsStrip !== false);
  const stats = useItemStats(kind, id, on);
  if (!on || !stats || stats === "error" || stats.plays === 0) return null;
  return (
    <section className="stats-strip" aria-label="Your stats" data-testid="stats-strip">
      <StatsStripView stats={stats} kind={kind} />
    </section>
  );
}
