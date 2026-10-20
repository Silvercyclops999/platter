/**
 * Wrapped and Top Songs on the Stats page's Wrapped tab: a ruled list, newest
 * first. Each year with plays has its Wrapped (this year's from December 1,
 * or any time with `current`) and its Top Songs list, then All-Time. Each
 * row is that year's record sleeve, the year and what opening it shows.
 */
import { ChevronRight } from "lucide-react";
import { backend } from "../lib/backend";
import type { TopSongsIndex } from "../lib/statsTypes";
import { useLoad } from "../lib/useLoad";
import { nav } from "../stores/router";
import { useSettings } from "../stores/settings";
import { wrapped } from "../stores/wrapped";
import { toneStyle, YearBanner } from "./TopSongsTile";
import "./ledger.css";

/** One year's row: its record sleeve, the year, what opening it shows. The You tab has this year's Wrapped as one. */
export function YearEntry({ year, title, shown, sub, onOpen, className = "" }: { year: number | null; title: string; shown: string; sub: string; onOpen: () => void; className?: string }) {
  return (
    <button type="button" className={`lg-year ${className}`} aria-label={title} onClick={onOpen}>
      <span className="lg-year__sleeve" style={toneStyle(year)} aria-hidden="true">
        <YearBanner year={year} w={96} h={96} />
      </span>
      <span className="lg-year__year lg-num">{year ?? ""}</span>
      <span className="lg-year__text">
        <span className="lg-year__title">{shown}</span>
        <span className="lg-year__sub">{sub}</span>
      </span>
      <ChevronRight className="lg-year__go" size={18} aria-hidden="true" />
    </button>
  );
}

function Entry(p: { year: number | null; title: string; shown: string; sub: string; onOpen: () => void }) {
  return (
    <li>
      <YearEntry {...p} />
    </li>
  );
}

/** `current`: this year's Wrapped is listed even before December (the Stats tab). */
export function WrappedSection({ index, current }: { index?: TopSongsIndex; current?: number }) {
  const on = useSettings((s) => s.settings?.wrappedTopSongs !== false);
  const loaded = useLoad(() => (index ? Promise.resolve(index) : backend.topSongsIndex()), [index, on]);
  const ix = loaded.data;
  const wrappedYears = ix ? (current && !ix.wrappedYears.includes(current) ? [current, ...ix.wrappedYears] : ix.wrappedYears) : [];
  if (!on || !ix || (!wrappedYears.length && !ix.years.length && !ix.allTime)) return null;
  return (
    <section className="stats-section wrapped-section" aria-labelledby="stats-wrapped" data-shot="wrapped">
      <div className="lg-head">
        <h2 id="stats-wrapped" className="lg-head__title stats-section__title">
          Wrapped and Top Songs
        </h2>
      </div>
      {wrappedYears.length > 0 && (
        <ul className="lg-years" aria-label="Wrapped">
          {wrappedYears.map((y) => (
            <Entry key={y} year={y} title={`${y} Wrapped`} shown="Wrapped" sub="Your year in records, songs and hours" onOpen={() => wrapped.open(y)} />
          ))}
        </ul>
      )}
      <ul className="lg-years" aria-label="Top Songs">
        {ix.years.map((y) => (
          <Entry key={y} year={y} title={`Your Top Songs ${y}`} shown="Your Top Songs" sub="Ranked by plays" onOpen={() => nav.go({ name: "topSongs", year: y })} />
        ))}
        {ix.allTime && <Entry year={null} title="All-Time Top Songs" shown="All-Time Top Songs" sub="With your play counts" onOpen={() => nav.go({ name: "topSongs" })} />}
      </ul>
    </section>
  );
}
