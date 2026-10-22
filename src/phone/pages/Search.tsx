/**
 * Search on the phone: one field, recent searches you can remove one by
 * one, and results as you type (no submit), the top artist first. The
 * same folding and typo rules as on the desktop: the core does the search.
 */
import { MoreHorizontal, Search as SearchIcon, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Art } from "../../components/Art";
import { backend } from "../../lib/backend";
import type { Album, CatArtist, SearchHit, SearchResults, Track } from "../../lib/types";
import { headerColorsFromServer } from "../../lib/color";
import { useSettings } from "../../stores/settings";
import { hitInfo, hitOf, lyricQuote, mixedHits, openHit, rememberHit, subtitleWithType } from "../../shell/searchModel";
import { useLoad } from "../../lib/useLoad";
import { nav, useRouter } from "../../stores/router";
import { phone } from "../phoneUi";
import { RowGlyphs, tap } from "../bits";
import { SongSwipeUnder, useRowSwipe } from "../rowSwipe";
import { isLiked, setLiked } from "../../stores/likes";
import { player, usePlayer } from "../../stores/player";
import { PlayingBars, countLine } from "./common";
import { catalogueTop, hasCatalogue, hasCatalogueRows, playCatalogueSong, useCatalogueSearch, useTapPlays, type CatalogueResults } from "../../pages/SearchCatalogue";
import { useArtistPhoto } from "../../lib/artistPhotos";
import { useGetSpots } from "../../pages/lidarrShared";
import { catalogueKind } from "../../lib/format";
import "../../pages/catalogue.css";

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
  const cat = useCatalogueSearch(q, "all");
  const all = res ? phoneResults(res) : [];
  const kinds = SEARCH_KINDS.filter((k) => k.id === "all" || all.some((h) => hitInfo(h)?.kind === k.id));
  // A pill picked for an earlier search that has none of its kind now, or
  // whose pills are hidden: All.
  const shown: SearchKind = !res || (kinds.length > 2 && kinds.some((k) => k.id === kind)) ? kind : "all";
  const hits = ofKind(all, shown);
  // An artist you don't have, named exactly, is the top result (unless the
  // library has something named exactly).
  const catTop = shown === "all" || shown === "artist" ? catalogueTop(q, cat, res?.top, all[0]) : null;
  const nothingAnywhere = !hits.length && !catTop && !hasCatalogueRows(cat) && !cat?.pending.length;
  const getSpots = useGetSpots();
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
            <button key={k.id} type="button" className={`ph-chip ${shown === k.id ? "is-on" : ""}`} aria-pressed={shown === k.id} onClick={() => (tap("selection"), setKind(k.id))}>
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
        <section aria-label="Results" className="ph-search__results">
          {(catTop || hits.length > 0) && (
            <ul className="ph-rows">
              {catTop && <CatalogueArtistRow artist={catTop} get={getSpots} />}
              {hits.map((h) => {
                const versions = h.track ? versionsCount(h.track, res) : 0;
                return (
                  <HitRow
                    key={hitInfo(h)?.key}
                    hit={h}
                    typed={shown !== "all"}
                    onOpen={() => open(h)}
                    // Versions are a line in the song's sheet, not a button on every row.
                    onMore={h.track ? () => phone.push({ kind: "track", track: { ...h.track!, otherVersions: Math.max(h.track!.otherVersions ?? 0, versions) } }) : h.album ? () => phone.push({ kind: "album", album: h.album! }) : undefined}
                    versions={versions}
                    lyric={h.track ? res.lyricLines?.[h.track.id] : undefined}
                  />
                );
              })}
            </ul>
          )}
          {nothingAnywhere && <p className="ph-rows__note">Nothing in your library matches "{q}".</p>}
          <PhoneCatalogue c={cat} kind={shown} skipArtist={catTop?.mbid} getSpots={getSpots} />
        </section>
      )}
    </div>
  );
}

/**
 * How many other versions a song has (deluxe, live, alternate, V1 and V2):
 * the ones search folded under it, else the count the library keeps.
 */
export function versionsCount(t: Track, res: SearchResults | null, countsOn = useSettings.getState().settings?.showVersionCounts !== false): number {
  const folded = res?.versions?.[t.id]?.length ?? 0;
  if (folded) return folded;
  return countsOn ? (t.otherVersions ?? 0) : 0;
}

/** A result's grey line: "Album · Artist" among all results; under a type pill the type goes (the pill says it). */
export function hitLine(i: NonNullable<ReturnType<typeof hitInfo>>, typed: boolean): string {
  return typed ? i.byline || i.typeLabel : subtitleWithType(i);
}

/** One row style for every kind of result, in every pill, so switching pills keeps the list's look. */
function HitRow({
  hit,
  top,
  typed = false,
  onOpen,
  onRemove,
  onMore,
  versions = 0,
  lyric,
}: {
  hit: SearchHit;
  top?: boolean;
  typed?: boolean;
  onOpen: () => void;
  onRemove?: () => void;
  onMore?: () => void;
  versions?: number;
  /** The lyric line the song was found by. */
  lyric?: string;
}) {
  const i = hitInfo(hit);
  // A song swipes right into the queue, and left into (or out of) Liked Songs.
  const song = hit.track;
  const swipe = useRowSwipe(
    song
      ? {
          right: () => void player.addToQueue([song]),
          left: () => void setLiked([song], !isLiked(song.id)),
        }
      : {},
  );
  // The playing song wears the level bars.
  const playing = usePlayer((s) => !!hit.track && s.state?.track?.id === hit.track.id);
  if (!i) return null;
  return (
    <li
      ref={song ? swipe.ref : undefined}
      className={`ph-hit ${song ? swipe.className : ""} ${top ? "is-top" : ""}`}
      {...(song ? swipe.props : {})}
    >
      {song && <SongSwipeUnder trackId={song.id} />}
      <button type="button" className="ph-row" onClick={onOpen}>
        <Art path={i.thumb} size={52} round={i.round} kind={i.kind === "track" ? "album" : i.kind} monogram={i.round ? i.title : undefined} />
        <span className="ph-row__text">
          <span className={`ph-row__title ${playing ? "is-playing" : ""}`}>
            {playing && <PlayingBars />}
            {i.title}
          </span>
          <span className="ph-row__sub">
            <RowGlyphs trackId={i.kind === "track" ? i.id : undefined} albumId={i.kind === "album" ? i.id : undefined} />
            {hitLine(i, typed)}
            {versions > 0 && hit.track && <span className="ph-row__versions"> · {versions + 1} versions</span>}
          </span>
          {lyric && <span className="ph-row__lyric">{lyricQuote(lyric)}</span>}
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

/** Spine heights in a crate divider, so the records stand a little uneven. */
const SPINES = [44, 40, 46, 42, 38];

/** Spine colours for a record whose cover colour isn't known yet: old cloth bindings. */
const CLOTH = ["#7b2b33", "#2f6e4a", "#3c3586", "#b07a3a", "#1f3f6e", "#6a4b8e", "#8f5a2b"];

/** A record's spine colour: its cover's, or a cloth colour picked by its id. */
export function spineColour(a: Album): string {
  const own = headerColorsFromServer(a.tint)?.vivid;
  if (own) return own;
  let h = 0;
  for (const c of a.id) h = (h * 31 + c.charCodeAt(0)) | 0;
  return CLOTH[Math.abs(h) % CLOTH.length];
}

/**
 * With nothing typed: the library's genres to browse, as crate dividers.
 * A tab with the genre's name, how many records, and the spines of the
 * records inside in their cover colours.
 */
function Browse() {
  const { data } = useLoad(() => backend.crates("genre"), []);
  const genres = (data?.crates ?? []).slice(0, 8);
  if (!genres.length) return null;
  return (
    <section aria-labelledby="ph-browse">
      <h2 id="ph-browse" className="ph-section-title">
        Browse the crates
      </h2>
      <ul className="ph-browse">
        {genres.map((g) => (
          <li key={g.key}>
            <button type="button" className="ph-divider" data-testid="crate-divider" onClick={() => nav.go({ name: "crate", kind: g.kind, key: g.key })}>
              <span className="ph-divider__tab">{g.name}</span>
              <span className="ph-divider__body">
                <span className="ph-divider__count">{countLine(g.count, "record")}</span>
                <span className="ph-divider__spines" aria-hidden="true">
                  {g.albums.slice(0, SPINES.length).map((a, i) => (
                    <span key={a.id} style={{ height: SPINES[i], background: spineColour(a) }} />
                  ))}
                </span>
              </span>
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}

/** An artist you don't have, as a result row: their photo, and their page (Get albums) a tap away. */
function CatalogueArtistRow({ artist, get }: { artist: CatArtist; get: boolean }) {
  const box = useRef<HTMLLIElement>(null);
  const { photo } = useArtistPhoto(box, artist.mbid, artist.name, artist.photo);
  return (
    <li ref={box} className="ph-hit">
      <button type="button" className="ph-row" aria-label={`${artist.name}, artist, not in your library`} onClick={() => nav.go({ name: "catalogueArtist", mbid: artist.mbid })}>
        <Art path={photo} size={52} round kind="artist" monogram={artist.name} />
        <span className="ph-row__text">
          <span className="ph-row__title">{artist.name}</span>
          <span className="ph-row__sub">Artist · Not in your library</span>
        </span>
        {get && (
          <span className="sr-get-mark" aria-hidden="true">
            Get
          </span>
        )}
      </button>
    </li>
  );
}

/**
 * Music you don't have, under the library's results: artists first, then a
 * song plays on tap when Lidarr can get it (Get where the length would be),
 * then records; an album or an artist opens its page, with Get there. The
 * artist shown as the top result isn't listed again.
 */
function PhoneCatalogue({ c, kind, skipArtist, getSpots }: { c: CatalogueResults | null; kind: SearchKind; skipArtist?: string; getSpots: boolean }) {
  const tapPlays = useTapPlays(getSpots);
  if (!c || !hasCatalogue(c)) return null;
  const songs = kind === "all" || kind === "track" ? c.songs : [];
  const albums = kind === "all" || kind === "album" ? c.albums : [];
  const artists = kind === "all" || kind === "artist" ? c.artists.filter((a) => !a.libraryId && a.mbid !== skipArtist) : [];
  if (!songs.length && !albums.length && !artists.length) return null;
  return (
    <section aria-label="Not in your library" className="ph-search__catalogue">
      <h2 className="ph-section-title">Not in your library</h2>
      <ul className="ph-rows">
        {artists.map((a) => (
          <CatalogueArtistRow key={`ar-${a.mbid}`} artist={a} get={getSpots} />
        ))}
        {songs.map((s) => (
          <li key={`so-${s.recordingId}`} className="ph-hit">
            <button
              type="button"
              className="ph-row"
              aria-label={`${s.title}, song by ${s.artist}${tapPlays ? ", a tap gets it and plays it" : ""}`}
              onClick={() => (tapPlays ? void playCatalogueSong(s) : s.releaseGroupId && nav.go({ name: "catalogueAlbum", id: s.releaseGroupId }))}
            >
              <Art path={s.thumb} size={52} kind="album" />
              <span className="ph-row__text">
                <span className="ph-row__title">{s.title}</span>
                <span className="ph-row__sub">{[s.artist, s.album].filter(Boolean).join(" · ")}</span>
              </span>
              {tapPlays && (
                <span className="sr-get-mark" aria-hidden="true">
                  Get
                </span>
              )}
            </button>
          </li>
        ))}
        {albums.map((a) => (
          <li key={`al-${a.releaseGroupId}`} className="ph-hit">
            <button type="button" className="ph-row" onClick={() => nav.go({ name: "catalogueAlbum", id: a.releaseGroupId })}>
              <Art path={a.thumb} size={52} kind="album" />
              <span className="ph-row__text">
                <span className="ph-row__title">{a.title}</span>
                <span className="ph-row__sub">{[catalogueKind(a), a.artist].filter(Boolean).join(" · ")}</span>
              </span>
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}
