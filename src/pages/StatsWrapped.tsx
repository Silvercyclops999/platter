/**
 * Wrapped and Top Songs on the Stats page's Recaps: a ruled list, newest
 * first. Each year with plays has its Wrapped (this year's from December 1;
 * the app menu has it any time) and its Top Songs list, then All-Time. Each
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

function Entry({ year, title, shown, sub, onOpen }: { year: number | null; title: string; shown: string; sub: string; onOpen: () => void }) {
  return (
    <li>
      <button type="button" className="year-entry" aria-label={title} onClick={onOpen}>
        <span className="year-entry__sleeve" style={toneStyle(year)} aria-hidden="true">
          <YearBanner year={year} w={96} h={96} />
        </span>
        <span className="year-entry__year num">{year ?? ""}</span>
        <span className="year-entry__text">
          <span className="year-entry__title">{shown}</span>
          <span className="year-entry__sub">{sub}</span>
        </span>
        <ChevronRight className="year-entry__go" size={18} aria-hidden="true" />
      </button>
    </li>
  );
}

export function WrappedSection({ index }: { index?: TopSongsIndex }) {
  const on = useSettings((s) => s.settings?.wrappedTopSongs !== false);
  const loaded = useLoad(() => (index ? Promise.resolve(index) : backend.topSongsIndex()), [index, on]);
  const ix = loaded.data;
  if (!on || !ix || (!ix.wrappedYears.length && !ix.years.length && !ix.allTime)) return null;
  return (
    <section className="stats-section wrapped-section" aria-labelledby="stats-wrapped" data-shot="wrapped">
      <div className="ledger-head">
        <h2 id="stats-wrapped" className="ledger-head__title stats-section__title">
          Wrapped and Top Songs
        </h2>
      </div>
      {ix.wrappedYears.length > 0 && (
        <ul className="year-entries" aria-label="Wrapped">
          {ix.wrappedYears.map((y) => (
            <Entry key={y} year={y} title={`${y} Wrapped`} shown="Wrapped" sub="Your year in records, songs and hours" onOpen={() => wrapped.open(y)} />
          ))}
        </ul>
      )}
      <ul className="year-entries" aria-label="Top Songs">
        {ix.years.map((y) => (
          <Entry key={y} year={y} title={`Your Top Songs ${y}`} shown="Your Top Songs" sub="Ranked by plays" onOpen={() => nav.go({ name: "topSongs", year: y })} />
        ))}
        {ix.allTime && <Entry year={null} title="All-Time Top Songs" shown="All-Time Top Songs" sub="With your play counts" onOpen={() => nav.go({ name: "topSongs" })} />}
      </ul>
    </section>
  );
}
