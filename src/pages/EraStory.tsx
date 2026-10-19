/**
 * The top of an era page, on the computer and the phone: the era in one
 * sentence (when it ran, how many songs over how many releases, how many of
 * them unreleased), then its releases on a timeline from the first to the
 * last, the newest marked. With fewer than two dated releases there is no
 * span to draw, so only the sentence shows.
 */
import { plural } from "../lib/format";
import type { EraPage } from "../lib/types";
import { eraRange, formatArchiveDate } from "./era";
import "./ledger.css";

/** Months since year 0 from "2019", "2019-02" or "2019-02-11"; null for anything else. */
export function monthIndex(d: string | null | undefined): number | null {
  const m = /^(\d{4})(?:-(\d{2}))?(?:-(\d{2}))?/.exec(d?.trim() ?? "");
  if (!m) return null;
  const month = m[2] ? Number(m[2]) - 1 : 0;
  const day = m[3] ? (Number(m[3]) - 1) / 31 : 0;
  return Number(m[1]) * 12 + month + day;
}

export interface EraMark {
  id: string;
  title: string;
  date: string;
  /** 0 to 1 along the span. */
  at: number;
  /** Which line its label sits on, so close ones don't overlap. */
  row: number;
  newest: boolean;
}

/** The releases placed along the era's span, oldest first, each label on the first line with room. */
export function eraMarks(groups: EraPage["groups"], gap = 0.28): EraMark[] {
  const dated = groups
    .map((g, i) => {
      const date = g.date ?? (g.album.year ? String(g.album.year) : null);
      return { g, i, date, m: monthIndex(date) };
    })
    .filter((x): x is typeof x & { date: string; m: number } => x.m != null);
  if (dated.length < 2) return [];
  const min = Math.min(...dated.map((x) => x.m));
  const max = Math.max(...dated.map((x) => x.m));
  if (max <= min) return [];
  const newest = dated.reduce((a, b) => (b.m > a.m ? b : a));
  const sorted = [...dated].sort((a, b) => a.m - b.m || b.i - a.i);
  const lastAt: number[] = [];
  return sorted.map((x) => {
    const at = (x.m - min) / (max - min);
    let row = lastAt.findIndex((p) => at - p >= gap);
    if (row < 0) row = lastAt.length < 3 ? lastAt.length : 0;
    lastAt[row] = at;
    return { id: x.g.album.id, title: x.g.album.title, date: formatArchiveDate(x.date), at, row, newest: x === newest };
  });
}

/** "Glass Season ran from Sep 2018 to Feb 2019: 15 songs over 3 releases, 6 of them unreleased." */
export function eraSentence(data: EraPage): { lead: string; rest: string } {
  const era = data.era;
  const range = eraRange(era);
  const unreleased = data.groups.reduce((n, g) => n + g.tracks.filter((t) => t.songStatus && t.songStatus !== "released").length, 0);
  const when = !range ? " holds" : range.includes(" to ") ? ` ran from ${range}:` : ` is from ${range}:`;
  const count = `${plural(era.trackCount, "song")} over ${plural(era.albumCount, "release")}`;
  const unrel = unreleased === 0 ? "" : unreleased >= era.trackCount ? ", all of them unreleased" : `, ${unreleased.toLocaleString()} of them unreleased`;
  return { lead: era.name, rest: `${when} ${count}${unrel}.` };
}

/** `gap`: how far apart (0 to 1) two labels must be to share a line; the phone needs more room. */
export function EraStory({ data, className = "", gap }: { data: EraPage; className?: string; gap?: number }) {
  const s = eraSentence(data);
  const marks = eraMarks(data.groups, gap);
  const rows = marks.length ? Math.max(...marks.map((m) => m.row)) + 1 : 0;
  const first = marks[0];
  const last = marks[marks.length - 1];
  return (
    <div className={`era-story ${className}`} data-testid="era-story">
      <p className="lg-lead lg-lead--sm era-story__sentence">
        <strong>{s.lead}</strong>
        {s.rest}
      </p>
      {marks.length > 0 && (
        <figure
          className="era-line"
          role="img"
          aria-label={`Releases from ${first.date} to ${last.date}: ${marks.map((m) => `${m.title}, ${m.date}`).join("; ")}.`}
          style={{ "--era-rows": rows } as React.CSSProperties}
        >
          <span className="era-line__rule" aria-hidden="true" />
          {marks.map((m) => (
            <span
              key={m.id}
              className={`era-line__mark ${m.newest ? "is-newest" : ""} ${m.at > 0.7 ? "is-end" : ""}`}
              style={{ left: `${m.at * 100}%`, "--row": m.row } as React.CSSProperties}
              aria-hidden="true"
            >
              <span className="era-line__tick" />
              <span className="era-line__label">
                <span className="era-line__title">{m.title}</span>
                <span className="era-line__date lg-num">{m.date}</span>
              </span>
            </span>
          ))}
        </figure>
      )}
    </div>
  );
}
