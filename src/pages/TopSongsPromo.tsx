/**
 * Home's one place for Top Songs: the new year's list from early December
 * to mid January (the core decides the window; `promoted` is null outside
 * it). Otherwise Top Songs lists stay off Home.
 */
import { Sparkles } from "lucide-react";
import { Button } from "../components/Button";
import { backend } from "../lib/backend";
import { useLoad } from "../lib/useLoad";
import { nav } from "../stores/router";
import { useSettings } from "../stores/settings";
import { wrapped } from "../stores/wrapped";
import { toneStyle, YearBanner } from "./TopSongsTile";

export function TopSongsPromo() {
  const on = useSettings((s) => s.settings?.wrappedTopSongs !== false);
  const { data: ix } = useLoad(() => (on ? backend.topSongsIndex() : Promise.resolve(null)), [on]);
  const year = ix?.promoted ?? null;
  if (!on || year == null) return null;
  const title = `Your Top Songs ${year}`;
  return (
    <section className="ts-promo" style={toneStyle(year)} aria-label={title} data-testid="top-songs-promo">
      <YearBanner year={year} w={640} h={240} />
      <div className="ts-promo__text">
        <span className="ts-promo__year" aria-hidden="true">
          {year}
        </span>
        <div>
          <h2 className="ts-promo__title">{title} is here</h2>
          <p className="ts-promo__sub">The songs you loved most this year, ranked by your plays.</p>
          <div className="ts-promo__actions">
            <Button variant="primary" size="sm" onClick={() => nav.go({ name: "topSongs", year })}>
              Open the list
            </Button>
            <Button size="sm" icon={<Sparkles size={14} />} onClick={() => wrapped.open(year)}>
              {year} Wrapped
            </Button>
          </div>
        </div>
      </div>
    </section>
  );
}
