/**
 * The year's record art (a pattern seeded by the year) and the tile that
 * opens a Top Songs list from search, the Stats page and Home.
 */
import { useMemo, type CSSProperties } from "react";
import { LABEL_RING, yearPattern, yearTone } from "../lib/wrapped";
import { nav } from "../stores/router";
import "./top-songs.css";

const BANNER_W = 1200;
const BANNER_H = 360;

/** The year's record, drawn as SVG. Sized by its box (it fills and crops). */
export function YearBanner({ year, w = BANNER_W, h = BANNER_H }: { year: number | null; w?: number; h?: number }) {
  const p = useMemo(() => yearPattern(year, w, h), [year, w, h]);
  return (
    <svg className="year-art" viewBox={`0 0 ${w} ${h}`} preserveAspectRatio="xMidYMid slice" aria-hidden="true" data-bands={p.bands}>
      <circle cx={p.cx} cy={p.cy} r={p.radius} className="year-art__disc" />
      {p.grooves.map((g) => (
        <circle key={g.r} cx={p.cx} cy={p.cy} r={g.r} className="year-art__groove" style={{ opacity: g.alpha }} />
      ))}
      <circle cx={p.cx} cy={p.cy} r={p.label} className="year-art__label" />
      <circle cx={p.cx} cy={p.cy} r={p.label * LABEL_RING} className="year-art__ring" />
      <circle cx={p.cx} cy={p.cy} r={p.hole} className="year-art__hole" />
    </svg>
  );
}

export const toneStyle = (year: number | null) => {
  const t = yearTone(year);
  return { "--year-deep": t.deep, "--year-light": t.light } as CSSProperties;
};

/** A square tile for a list, with its name and a line under it. */
export function TopSongsTile({ year, sub }: { year: number | null; sub?: string }) {
  const title = year == null ? "All-Time Top Songs" : `Your Top Songs ${year}`;
  return (
    <div className="ts-card">
      <button type="button" className="ts-tile" style={toneStyle(year)} aria-label={title} onClick={() => nav.go(year == null ? { name: "topSongs" } : { name: "topSongs", year })}>
        <YearBanner year={year} w={400} h={400} />
        <span className="ts-tile__kicker">{year == null ? "All-Time" : "Your"} Top Songs</span>
        <span className={`ts-tile__year ${year == null ? "ts-tile__year--words" : ""}`}>{year ?? "All time"}</span>
      </button>
      <span className="ts-card__title truncate">{title}</span>
      {sub && <span className="ts-card__sub truncate">{sub}</span>}
    </div>
  );
}
