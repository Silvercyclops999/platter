/**
 * Search on the phone: one field, recent searches you can remove one by
 * one, and results as you type (no submit), the top artist first. The
 * same folding and typo rules as on the desktop: the core does the search.
 */
import { MoreHorizontal, Search as SearchIcon, X } from "lucide-react";
import { useEffect, useState } from "react";
import { Art } from "../../components/Art";
import { backend } from "../../lib/backend";
import type { SearchHit, SearchResults } from "../../lib/types";
import { hitInfo, hitOf, mixedHits, openHit, rememberHit, subtitleWithType } from "../../shell/searchModel";
import { useRouter } from "../../stores/router";
import { phone } from "../phoneUi";

/** Typing settles for this long before the search runs. */
export const SEARCH_SETTLE_MS = 150;

/** The results in order: the top artist first, then the rest mixed. */
export function phoneResults(res: SearchResults): SearchHit[] {
  const artist = res.artists[0] ? hitOf.artist(res.artists[0]) : null;
  const top = artist ?? res.top ?? null;
  const rest = mixedHits({ ...res, top }).filter((h) => !top || hitInfo(h)?.key !== hitInfo(top)?.key);
  return top ? [top, ...rest] : rest;
}

export function PhoneSearch() {
  const route = useRouter((s) => s.entries[s.index].route);
  const initial = route.name === "search" ? (route.query ?? "") : "";
  const [query, setQuery] = useState(initial);
  const [res, setRes] = useState<SearchResults | null>(null);
  const [recents, setRecents] = useState<SearchHit[] | null>(null);
  const q = query.trim();

  useEffect(() => {
    let alive = true;
    backend.searchRecents().then((r) => alive && setRecents(r), () => alive && setRecents([]));
    return () => {
      alive = false;
    };
  }, []);

  useEffect(() => {
    if (!q) return setRes(null);
    let alive = true;
    const t = setTimeout(() => {
      backend.search({ query: q, filter: "all", limit: 20 }).then(
        (r) => alive && setRes(r),
        () => alive && setRes({ songs: [], artists: [], albums: [], playlists: [], tookMs: 0, source: "local" }),
      );
      useRouter.getState().replace({ name: "search", query: q });
    }, SEARCH_SETTLE_MS);
    return () => {
      alive = false;
      clearTimeout(t);
    };
  }, [q]);

  const open = (h: SearchHit) => {
    rememberHit(h);
    openHit(h);
  };
  const forget = (h: SearchHit) => {
    const i = hitInfo(h);
    if (!i) return;
    setRecents((r) => (r ?? []).filter((x) => hitInfo(x)?.key !== i.key));
    backend.searchRecentRemove(i.kind, i.id).catch(() => {});
  };
  const hits = res ? phoneResults(res) : [];
  return (
    <div className="ph-page ph-search">
      <h1 className="ph-title">Search</h1>
      <div className="ph-field">
        <SearchIcon size={20} aria-hidden="true" />
        <input
          type="search"
          className="ph-field__input"
          placeholder="Artists, records, songs"
          aria-label="Search your library"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          autoCapitalize="none"
          autoCorrect="off"
          spellCheck={false}
        />
        {query && (
          <button type="button" className="ph-icon-btn" aria-label="Clear search" onClick={() => setQuery("")}>
            <X size={18} aria-hidden="true" />
          </button>
        )}
      </div>
      {!q && (
        <section aria-label="Recent searches">
          {recents && recents.length > 0 ? (
            <>
              <h2 className="ph-section-title">Recent searches</h2>
              <ul className="ph-rows">
                {recents.map((h) => (
                  <HitRow key={hitInfo(h)?.key} hit={h} onOpen={() => open(h)} onRemove={() => forget(h)} />
                ))}
              </ul>
            </>
          ) : (
            recents && <p className="ph-rows__note">Search your library. What you open shows here next time.</p>
          )}
        </section>
      )}
      {q && res && (
        <section aria-label="Results">
          {hits.length ? (
            <ul className="ph-rows">
              {hits.map((h, i) => (
                <HitRow key={hitInfo(h)?.key} hit={h} top={i === 0} onOpen={() => open(h)} onMore={h.track ? () => phone.push({ kind: "track", track: h.track! }) : undefined} />
              ))}
            </ul>
          ) : (
            <p className="ph-rows__note">Nothing in your library matches "{q}".</p>
          )}
        </section>
      )}
    </div>
  );
}

function HitRow({ hit, top, onOpen, onRemove, onMore }: { hit: SearchHit; top?: boolean; onOpen: () => void; onRemove?: () => void; onMore?: () => void }) {
  const i = hitInfo(hit);
  if (!i) return null;
  return (
    <li className={`ph-hit ${top ? "is-top" : ""}`}>
      <button type="button" className="ph-row" onClick={onOpen}>
        <Art path={i.thumb} size={top ? 64 : 52} round={i.round} kind={i.kind === "track" ? "album" : i.kind} monogram={i.round ? i.title : undefined} />
        <span className="ph-row__text">
          <span className="ph-row__title">{i.title}</span>
          <span className="ph-row__sub">{subtitleWithType(i)}</span>
        </span>
      </button>
      {onRemove && (
        <button type="button" className="ph-icon-btn" aria-label={`Remove ${i.title} from recent searches`} onClick={onRemove}>
          <X size={18} aria-hidden="true" />
        </button>
      )}
      {onMore && !onRemove && (
        <button type="button" className="ph-icon-btn ph-hit__more" aria-label={`More for ${i.title}`} onClick={onMore}>
          <MoreHorizontal size={20} aria-hidden="true" />
        </button>
      )}
    </li>
  );
}
