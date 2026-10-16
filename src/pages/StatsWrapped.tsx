/**
 * The profile's Wrapped and Top Songs: a Wrapped for each year with plays
 * (this year's from December 1; the app menu has it any time) and each
 * year's Top Songs list, newest first, then All-Time.
 */
import { Sparkles } from "lucide-react";
import { backend } from "../lib/backend";
import type { TopSongsIndex } from "../lib/statsTypes";
import { useLoad } from "../lib/useLoad";
import { useSettings } from "../stores/settings";
import { wrapped } from "../stores/wrapped";
import { toneStyle, TopSongsTile, YearBanner } from "./TopSongsTile";

function WrappedTile({ year }: { year: number }) {
  return (
    <button type="button" className="wrapped-tile" style={toneStyle(year)} onClick={() => wrapped.open(year)}>
      <YearBanner year={year} w={640} h={280} />
      <span className="wrapped-tile__text">
        <span className="wrapped-tile__year">{year}</span>
        <span className="wrapped-tile__name">
          <Sparkles size={14} aria-hidden="true" />
          Wrapped
        </span>
      </span>
    </button>
  );
}

export function WrappedSection({ index }: { index?: TopSongsIndex }) {
  const on = useSettings((s) => s.settings?.wrappedTopSongs !== false);
  const loaded = useLoad(() => (index ? Promise.resolve(index) : backend.topSongsIndex()), [index, on]);
  const ix = loaded.data;
  if (!on || !ix || (!ix.wrappedYears.length && !ix.years.length && !ix.allTime)) return null;
  return (
    <section className="stats-section wrapped-section" aria-labelledby="stats-wrapped" data-shot="wrapped">
      <h2 id="stats-wrapped" className="stats-section__title">
        Wrapped and Top Songs
      </h2>
      {ix.wrappedYears.length > 0 && (
        <ul className="wrapped-section__years" aria-label="Wrapped">
          {ix.wrappedYears.map((y) => (
            <li key={y}>
              <WrappedTile year={y} />
            </li>
          ))}
        </ul>
      )}
      <ul className="wrapped-section__lists" aria-label="Top Songs">
        {ix.years.map((y) => (
          <li key={y}>
            <TopSongsTile year={y} sub="Ranked by plays" />
          </li>
        ))}
        {ix.allTime && (
          <li>
            <TopSongsTile year={null} sub="With your play counts" />
          </li>
        )}
      </ul>
    </section>
  );
}
