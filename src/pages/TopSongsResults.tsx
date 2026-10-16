/** Top Songs lists in search, for "top songs", "wrapped", "top songs 2025" and the like. */
import { backend } from "../lib/backend";
import { useLoad } from "../lib/useLoad";
import { asksForTopSongs, topSongsMatches } from "../lib/wrapped";
import { useSettings } from "../stores/settings";
import { TopSongsTile } from "./TopSongsTile";

/** The lists a query finds (null: All-Time); empty unless the words are about them. */
export function useTopSongsFound(query: string): (number | null)[] {
  const on = useSettings((s) => s.settings?.wrappedTopSongs !== false);
  // The index is only asked for when the words are about these lists.
  const could = on && asksForTopSongs(query);
  const { data: ix } = useLoad(() => (could ? backend.topSongsIndex() : Promise.resolve(null)), [could]);
  return could && ix ? topSongsMatches(query, ix) : [];
}

export function TopSongsResults({ found }: { found: (number | null)[] }) {
  if (!found.length) return null;
  return (
    <section className="sr-top-songs" aria-labelledby="sr-top-songs">
      <h2 id="sr-top-songs" className="section-title">
        Your Top Songs
      </h2>
      <ul className="wrapped-section__lists">
        {found.slice(0, 8).map((y) => (
          <li key={y ?? "all"}>
            <TopSongsTile year={y} sub="Ranked by plays" />
          </li>
        ))}
      </ul>
    </section>
  );
}
