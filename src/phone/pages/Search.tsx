/**
 * Search on the phone: one field, recent searches you can remove one by
 * one, and results as you type (no submit), the top artist first. The
 * same folding and typo rules as on the desktop: the core does the search.
 */
import { MoreHorizontal, Search as SearchIcon, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Art } from "../../components/Art";
import { backend } from "../../lib/backend";
import type { SearchHit, SearchResults } from "../../lib/types";
import { hitInfo, hitOf, mixedHits, openHit, rememberHit, subtitleWithType } from "../../shell/searchModel";
import { useLoad } from "../../lib/useLoad";
import { nav, useRouter } from "../../stores/router";
import { phone } from "../phoneUi";
import { tap } from "../bits";

/** Typing settles for this long before the search runs. */
export const SEARCH_SETTLE_MS = 150;

/** The results in order: the top artist first, then the rest mixed. */
export function phoneResults(res: SearchResults): SearchHit[] {
  const artist = res.artists[0] ? hitOf.artist(res.artists[0]) : null;
  const top = artist ?? res.top ?? null;
  const rest = mixedHits({ ...res, top }).filter((h) => !top || hitInfo(h)?.key !== hitInfo(top)?.key);
  return top ? [top, ...rest] : rest;
}

/** The type pills over the results. */
export const SEARCH_KINDS = [
  { id: "all", label: "All" },
  { id: "track", label: "Songs" },
  { id: "album", label: "Records" },
  { id: "artist", label: "Artists" },
  { id: "playlist", label: "Playlists" },
] as const;
export type SearchKind = (typeof SEARCH_KINDS)[number]["id"];

/** The results of one type (all of them for "all"). */
export function ofKind(hits: SearchHit[], kind: SearchKind): SearchHit[] {
  return kind === "all" ? hits : hits.filter((h) => hitInfo(h)?.kind === kind);
}

export function PhoneSearch() {
  const route = useRouter((s) => s.entries[s.index].route);
  const initial = route.name === "search" ? (route.query ?? "") : "";
  const [query, setQuery] = useState(initial);
  const [res, setRes] = useState<SearchResults | null>(null);
  const [recents, setRecents] = useState<SearchHit[] | null>(null);
  const [kind, setKind] = useState<SearchKind>("all");
  const [focused, setFocused] = useState(false);
  const input = useRef<HTMLInputElement>(null);
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
  const all = res ? phoneResults(res) : [];
  const hits = ofKind(all, kind);
  const kinds = SEARCH_KINDS.filter((k) => k.id === "all" || all.some((h) => hitInfo(h)?.kind === k.id));
  return (
    <div className="ph-page ph-search">
      <h1 className="ph-title">Search</h1>
      <div className="ph-search__bar">
      <div
        className="ph-field"
        onClick={(e) => {
          // Anywhere on the field puts the cursor in it, in the same tap.
          if (!(e.target as HTMLElement).closest("button")) input.current?.focus();
        }}
      >
        <SearchIcon size={20} aria-hidden="true" />
        <input
          type="search"
          className="ph-field__input"
          placeholder="Artists, records, songs"
          aria-label="Search your library"
          ref={input}
          value={query}
          enterKeyHint="search"
          onFocus={() => setFocused(true)}
          onBlur={() => setFocused(false)}
          onChange={(e) => setQuery(e.target.value)}
          autoCapitalize="none"
          autoCorrect="off"
          spellCheck={false}
        />
        {query && (
          <button
            type="button"
            className="ph-icon-btn"
            aria-label="Clear search"
            onPointerDown={(e) => e.preventDefault()}
            onClick={() => {
              setQuery("");
              input.current?.focus();
            }}
          >
            <X size={18} aria-hidden="true" />
          </button>
        )}
      </div>
      {(focused || query) && (
        <button
          type="button"
          className="ph-search__cancel"
          onPointerDown={(e) => e.preventDefault()}
          onClick={() => {
            setQuery("");
            input.current?.blur();
          }}
        >
          Cancel
        </button>
      )}
      </div>
      {q && kinds.length > 2 && (
        <div className="ph-pills ph-pills--scroll" role="group" aria-label="Show">
          {kinds.map((k) => (
            <button key={k.id} type="button" className={`ph-chip ${kind === k.id ? "is-on" : ""}`} aria-pressed={kind === k.id} onClick={() => (tap("selection"), setKind(k.id))}>
              {k.label}
            </button>
          ))}
        </div>
      )}
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
          ) : null}
          <Browse />
        </section>
      )}
      {q && res && (
        <section aria-label="Results">
          {hits.length ? (
            <ul className="ph-rows">
              {hits.map((h, i) => (
                <HitRow
                  key={hitInfo(h)?.key}
                  hit={h}
                  top={i === 0 && kind === "all"}
                  onOpen={() => open(h)}
                  onMore={h.track ? () => phone.push({ kind: "track", track: h.track! }) : h.album ? () => phone.push({ kind: "album", album: h.album! }) : undefined}
                />
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
        <Art path={i.thumb} size={52} round={i.round} kind={i.kind === "track" ? "album" : i.kind} monogram={i.round ? i.title : undefined} />
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
      {!onRemove &&
        (onMore ? (
          <button type="button" className="ph-icon-btn ph-hit__more" aria-label={`More for ${i.title}`} onClick={onMore}>
            <MoreHorizontal size={20} aria-hidden="true" />
          </button>
        ) : (
          !top && <span className="ph-icon-btn" aria-hidden="true" />
        ))}
    </li>
  );
}

/** With nothing typed: the library's genres to browse, as crates. */
function Browse() {
  const { data } = useLoad(() => backend.crates("genre"), []);
  const genres = (data?.crates ?? []).slice(0, 8);
  if (!genres.length) return null;
  return (
    <section aria-labelledby="ph-browse">
      <h2 id="ph-browse" className="ph-section-title">
        Browse your genres
      </h2>
      <ul className="ph-browse">
        {genres.map((g) => (
          <li key={g.key}>
            <button type="button" className="ph-browse__tile" onClick={() => nav.go({ name: "crate", kind: g.kind, key: g.key })}>
              <span className="ph-browse__name">{g.name}</span>
              <span className="ph-browse__count tabular">{g.count.toLocaleString()}</span>
              {g.albums[0] && (
                <span className="ph-browse__art" aria-hidden="true">
                  <Art path={g.albums[0].thumb} size={64} sleeve={{ title: g.albums[0].title, artist: g.albums[0].artistTitle }} />
                </span>
              )}
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}
