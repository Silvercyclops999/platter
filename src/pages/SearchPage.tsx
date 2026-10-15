import { ChevronDown, MoreHorizontal, Pause, Play } from "lucide-react";
import { Fragment, useEffect, useMemo, useRef, useState, type MouseEvent, type ReactNode } from "react";
import { Art } from "../components/Art";
import { CardSkeleton } from "../components/Card";
import { openContextMenu } from "../components/ContextMenu";
import { shareItem } from "../components/ShareMenu";
import { Skeleton } from "../components/Skeleton";
import { FilterChips } from "../components/Tabs";
import { TrackList, TrackRow, TrackRowSkeleton, trackMenuItems } from "../components/TrackRow";
import { backend } from "../lib/backend";
import { isOffline, OFFLINE_SEARCH } from "../lib/offline";
import { modKey } from "../lib/platform";
import type { SearchFilter, SearchHit, SearchResults } from "../lib/types";
import type { MenuItem } from "../stores/ui";
import { useApp } from "../stores/app";
import { player, usePlayer } from "../stores/player";
import { nav } from "../stores/router";
import { MAX_QUERY } from "../shell/SearchBox";
import { hitInfo, hitOf, mixedHits, openHit, playHit, rememberHit, type HitInfo } from "../shell/searchModel";
import { AlbumCard, ArtistCard, PlaylistCard, useContextState } from "./cards";
import { otherVersionsText } from "./era";
import { SearchCatalogue } from "./SearchCatalogue";
import "./search.css";

export { MAX_QUERY };

/** Results asked for per kind. */
const LIMIT = 30;

export const FILTERS: { id: SearchFilter; label: string }[] = [
  { id: "all", label: "All" },
  { id: "songs", label: "Songs" },
  { id: "albums", label: "Albums" },
  { id: "artists", label: "Artists" },
  { id: "playlists", label: "Playlists" },
];

// ---------------------------------------------------------------- rows

function menuFor(h: SearchHit, playing: boolean, toggle: () => void): MenuItem[] {
  const i = hitInfo(h)!;
  const play: MenuItem = { label: playing ? "Pause" : "Play", onSelect: toggle };
  if (h.kind === "track" && h.track) return trackMenuItems(h.track, { before: [play] });
  if (h.kind === "album" && h.album) {
    const a = h.album;
    return [
      play,
      { label: "Add to queue", onSelect: () => void player.queueAlbum(a.id) },
      { label: "Go to artist", onSelect: () => nav.go({ name: "artist", id: a.artistId }) },
      shareItem("album", a.id, a.thumb),
    ];
  }
  if (h.kind === "playlist") return [play, { label: "Add to queue", onSelect: () => void player.queuePlaylist(i.id) }, shareItem("playlist", i.id, i.thumb)];
  if (h.kind === "artist") return [play, shareItem("artist", i.id, i.thumb)];
  return [play];
}

/** Whether a result is what's playing (a song) or the playing context (the rest). */
function usePlayingState(i: HitInfo): { current: boolean; playing: boolean } {
  const ctx = useContextState(i.kind === "track" ? "tracks" : i.kind, i.id);
  const isTrack = usePlayer((s) => i.kind === "track" && s.state?.track?.id === i.id);
  const trackPlaying = usePlayer((s) => isTrack && s.state?.status === "playing");
  return i.kind === "track" ? { current: isTrack, playing: trackPlaying } : { current: ctx.isContext, playing: ctx.playing };
}

function useHitActions(h: SearchHit, songIds?: string[]) {
  const i = hitInfo(h)!;
  const { current, playing } = usePlayingState(i);
  const toggle = () => {
    rememberHit(h);
    if (current) void player.toggle();
    else void playHit(h, songIds);
  };
  const open = () => {
    rememberHit(h);
    void openHit(h);
  };
  const menu = (e: MouseEvent<HTMLElement>, at?: HTMLElement) => {
    e.preventDefault();
    openContextMenu(`Options for ${i.title}`, menuFor(h, playing, toggle), at ?? e);
  };
  return { i, current, playing, toggle, open, menu };
}

/** The highlighted first result: taller, larger art, a big play button. */
function TopResult({ hit, songIds }: { hit: SearchHit; songIds: string[] }) {
  const { i, playing, toggle, open, menu } = useHitActions(hit, songIds);
  return (
    <section className="sr-top" aria-label="Top result" onContextMenu={(e) => menu(e)}>
      <Art path={i.thumb} size={92} round={i.round} kind={i.kind === "artist" ? "artist" : "album"} />
      <div className="sr-top__text">
        <button type="button" className="sr-top__title" onClick={open}>
          <span className="truncate">{i.title}</span>
        </button>
        <div className="sr-top__sub truncate">
          <span className="sr-type">{i.typeLabel}</span>
          {i.byline && <> · {i.byline}</>}
        </div>
      </div>
      <button type="button" className="sr-top__play" aria-label={`${playing ? "Pause" : "Play"} ${i.title}`} onClick={toggle}>
        {playing ? <Pause size={22} fill="currentColor" aria-hidden="true" /> : <Play size={22} fill="currentColor" aria-hidden="true" />}
      </button>
    </section>
  );
}

/** One plain result row: art, title and byline, a grey type label, more options. */
export function ResultRow({ hit, songIds }: { hit: SearchHit; songIds?: string[] }) {
  const { i, current, playing, toggle, open, menu } = useHitActions(hit, songIds);
  const more = useRef<HTMLButtonElement>(null);
  return (
    <li className={`sr-row ${current ? "is-current" : ""}`} aria-label={`${i.title}, ${i.typeLabel}${i.byline ? `, ${i.byline}` : ""}`} onContextMenu={(e) => menu(e)} onDoubleClick={toggle}>
      <span className="sr-row__art">
        <Art path={i.thumb} size={48} round={i.round} kind={i.kind === "artist" ? "artist" : "album"} />
        <button
          type="button"
          className={`sr-row__play ${i.round ? "is-round" : ""}`}
          aria-label={`${playing ? "Pause" : "Play"} ${i.title}`}
          onClick={(e) => {
            e.stopPropagation();
            toggle();
          }}
        >
          {playing ? <Pause size={16} fill="currentColor" aria-hidden="true" /> : <Play size={16} fill="currentColor" aria-hidden="true" />}
        </button>
      </span>
      <span className="sr-row__text">
        <button type="button" className="sr-row__title" onClick={open}>
          <span className="truncate">{i.title}</span>
        </button>
        <span className="sr-row__by truncate">{i.byline || i.typeLabel}</span>
      </span>
      <span className="sr-type sr-row__type">{i.typeLabel}</span>
      <button ref={more} type="button" className="sr-row__more" aria-label={`More options for ${i.title}`} onClick={(e) => menu(e, more.current ?? undefined)}>
        <MoreHorizontal size={18} aria-hidden="true" />
      </button>
    </li>
  );
}

function Remember({ hit, children }: { hit: SearchHit; children: ReactNode }) {
  // Opening or playing a card remembers it as a recent search.
  return <div onClickCapture={() => rememberHit(hit)}>{children}</div>;
}

/** "N other versions" under a song that has them; expands in place. */
function VersionsToggle({ id, count, open, onToggle, as }: { id: string; count: number; open: boolean; onToggle: () => void; as: "li" | "row" }) {
  const button = (
    <button type="button" className="sr-versions__toggle" aria-expanded={open} aria-controls={`versions-${id}`} onClick={onToggle}>
      <ChevronDown size={14} aria-hidden="true" className={open ? "is-open" : ""} />
      {otherVersionsText(count)}
    </button>
  );
  return as === "li" ? (
    <li className="sr-versions">{button}</li>
  ) : (
    <div role="row" className="sr-versions">
      <span role="gridcell">{button}</span>
    </div>
  );
}

// ---------------------------------------------------------------- loading

function ResultsSkeleton({ filter }: { filter: SearchFilter }) {
  if (filter === "songs")
    return (
      <TrackList label="Loading songs">
        {Array.from({ length: 8 }, (_, i) => (
          <TrackRowSkeleton key={i} showArt />
        ))}
      </TrackList>
    );
  if (filter !== "all")
    return (
      <div className="sr-grid" aria-hidden="true">
        {Array.from({ length: 8 }, (_, i) => (
          <CardSkeleton key={i} kind={filter === "artists" ? "artist" : filter === "playlists" ? "playlist" : "album"} />
        ))}
      </div>
    );
  return (
    <div className="sr-all" aria-hidden="true">
      <Skeleton height={124} radius="card" />
      {Array.from({ length: 6 }, (_, i) => (
        <div key={i} className="sr-row sr-row--skeleton">
          <Skeleton width={48} height={48} />
          <span className="sr-row__text">
            <Skeleton width={180} height={14} />
            <Skeleton width={110} height={12} />
          </span>
        </div>
      ))}
    </div>
  );
}

// ---------------------------------------------------------------- page

interface Shown {
  query: string;
  filter: SearchFilter;
  res: SearchResults;
}

/** The full results page for a query: type chips, then the results for that type. */
export function SearchPage({ query }: { query: string }) {
  const [filter, setFilter] = useState<SearchFilter>("all");
  const [shown, setShown] = useState<Shown | null>(null);
  const [failed, setFailed] = useState(false);
  const seq = useRef(0);
  const nowTrackId = usePlayer((s) => s.state?.track?.id);
  const status = usePlayer((s) => s.state?.status);
  const q = query.trim().slice(0, MAX_QUERY);
  const offline = useApp((s) => isOffline(s.health));

  // Each request gets a sequence number and only the newest one may update
  // the screen, so a slow answer to an older query never replaces a newer one.
  useEffect(() => {
    const mine = ++seq.current;
    if (!q) {
      setShown(null);
      setFailed(false);
      return;
    }
    backend.search({ query: q, filter, limit: LIMIT }).then(
      (res) => {
        if (mine !== seq.current) return;
        setShown({ query: q, filter, res });
        setFailed(false);
      },
      () => {
        if (mine !== seq.current) return;
        setFailed(true);
      },
    );
    // Reconnecting searches again, so server answers come back by themselves.
  }, [q, filter, offline]);

  // Results for another chip would show the wrong layout: treat them as absent.
  const res = shown && shown.filter === filter ? shown.res : null;
  const songIds = useMemo(() => (res ? res.songs.map((s) => s.id) : []), [res]);
  const rows = useMemo(() => (res && filter === "all" ? mixedHits(res) : []), [res, filter]);
  // Songs whose other versions are showing (search groups versions under the released one).
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(() => new Set());
  const toggle = (id: string) =>
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  if (!q)
    return (
      <div className="page-pad sr">
        <div className="sr-empty">
          <h1 className="section-title">Search your library</h1>
          <p>Find artists, albums, songs and playlists on your server. Use the search box at the top, or press {modKey}+L.</p>
        </div>
      </div>
    );

  const localCount = !res
    ? 0
    : filter === "all"
      ? res.songs.length + res.albums.length + res.artists.length + res.playlists.length
      : filter === "songs"
        ? res.songs.length
        : filter === "albums"
          ? res.albums.length
          : filter === "artists"
            ? res.artists.length
            : res.playlists.length;

  const empty =
    res &&
    !res.top &&
    (filter === "all"
      ? !res.songs.length && !res.albums.length && !res.artists.length && !res.playlists.length
      : filter === "songs"
        ? !res.songs.length
        : filter === "albums"
          ? !res.albums.length
          : filter === "artists"
            ? !res.artists.length
            : !res.playlists.length);

  return (
    <div className="page-pad sr" aria-busy={!res && !failed}>
      <h1 className="sr-only">Results for “{q}”</h1>
      <FilterChips label="Filter results" items={FILTERS} value={filter} onChange={setFilter} className="sr-chips" scroll />
      {offline && (failed || res?.source === "server" || empty) ? (
        // Only the server could answer this: say why, and that it comes back.
        <div className="sr-empty" role="status">
          <h2 className="section-title">No results for “{q.length > 60 ? `${q.slice(0, 60)}…` : q}”</h2>
          <p>{OFFLINE_SEARCH}</p>
        </div>
      ) : !res ? (
        failed ? (
          <div className="sr-empty">
            <h2 className="section-title">Search isn't available right now</h2>
            <p>Try again in a moment.</p>
          </div>
        ) : (
          <ResultsSkeleton filter={filter} />
        )
      ) : empty ? (
        <>
          <div className="sr-empty">
            <h2 className="section-title">No results for “{q.length > 60 ? `${q.slice(0, 60)}…` : q}” in your library</h2>
            <p>Check the spelling, or try fewer or different keywords.</p>
          </div>
          <SearchCatalogue query={q} filter={filter} localCount={0} />
        </>
      ) : (
        <>
          {filter === "all" && (
            <div className="sr-all">
              {res.top && <TopResult hit={res.top} songIds={songIds} />}
              {res.top?.kind === "track" && res.top.track && !!res.versions?.[res.top.track.id]?.length && (
                <ul className="sr-list sr-list--top-versions" aria-label={`Other versions of ${res.top.track.title}`}>
                  <VersionsToggle
                    id={res.top.track.id}
                    count={res.versions[res.top.track.id].length}
                    open={expanded.has(res.top.track.id)}
                    onToggle={() => toggle(res.top!.track!.id)}
                    as="li"
                  />
                  {expanded.has(res.top.track.id) && (
                    <li id={`versions-${res.top.track.id}`} className="sr-versions__list">
                      <ul>
                        {res.versions[res.top.track.id].map((o) => (
                          <ResultRow key={o.id} hit={hitOf.track(o)} songIds={res.versions![res.top!.track!.id].map((x) => x.id)} />
                        ))}
                      </ul>
                    </li>
                  )}
                </ul>
              )}
              <ul className="sr-list" aria-label="Results">
                {rows.map((h) => {
                  const others = h.kind === "track" && h.track ? res.versions?.[h.track.id] : undefined;
                  if (!others?.length) return <ResultRow key={hitInfo(h)!.key} hit={h} songIds={songIds} />;
                  const id = h.track!.id;
                  const open = expanded.has(id);
                  return (
                    <Fragment key={hitInfo(h)!.key}>
                      <ResultRow hit={h} songIds={songIds} />
                      <VersionsToggle id={id} count={others.length} open={open} onToggle={() => toggle(id)} as="li" />
                      {open && (
                        <li id={`versions-${id}`} className="sr-versions__list">
                          <ul aria-label={`Other versions of ${h.track!.title}`}>
                            {others.map((o) => (
                              <ResultRow key={o.id} hit={hitOf.track(o)} songIds={others.map((x) => x.id)} />
                            ))}
                          </ul>
                        </li>
                      )}
                    </Fragment>
                  );
                })}
              </ul>
            </div>
          )}
          {filter === "songs" && (
            <TrackList label="Songs">
              {res.songs.map((t, i) => {
                const others = res.versions?.[t.id] ?? [];
                const open = expanded.has(t.id);
                return (
                  <Fragment key={t.id}>
                    <TrackRow
                      track={t}
                      index={i + 1}
                      showArt
                      active={t.id === nowTrackId}
                      playing={t.id === nowTrackId && status === "playing"}
                      onPlay={() => {
                        rememberHit(hitOf.track(t));
                        void player.playTracks(songIds, t.id, "search");
                      }}
                    />
                    {others.length > 0 && <VersionsToggle id={t.id} count={others.length} open={open} onToggle={() => toggle(t.id)} as="row" />}
                    {open &&
                      others.map((o, j) => (
                        <TrackRow
                          key={`${t.id}:${o.id}`}
                          rowKey={`${t.id}:${o.id}`}
                          track={o}
                          index={j + 1}
                          showArt
                          compact={false}
                          labelNote={`another version of ${t.title}`}
                          active={o.id === nowTrackId}
                          playing={o.id === nowTrackId && status === "playing"}
                          onPlay={() => void player.playTracks(others.map((x) => x.id), o.id, "search")}
                        />
                      ))}
                  </Fragment>
                );
              })}
            </TrackList>
          )}
          {filter === "albums" && (
            <div className="sr-grid" role="list" aria-label="Albums">
              {res.albums.map((a) => (
                <div role="listitem" key={a.id}>
                  <Remember hit={hitOf.album(a)}>
                    <AlbumCard album={a} />
                  </Remember>
                </div>
              ))}
            </div>
          )}
          {filter === "artists" && (
            <div className="sr-grid" role="list" aria-label="Artists">
              {res.artists.map((a) => (
                <div role="listitem" key={a.id}>
                  <Remember hit={hitOf.artist(a)}>
                    <ArtistCard artist={a} />
                  </Remember>
                </div>
              ))}
            </div>
          )}
          {filter === "playlists" && (
            <div className="sr-grid" role="list" aria-label="Playlists">
              {res.playlists.map((p) => (
                <div role="listitem" key={p.id}>
                  <Remember hit={hitOf.playlist(p)}>
                    <PlaylistCard playlist={p} />
                  </Remember>
                </div>
              ))}
            </div>
          )}
          <p className="sr-meta">
            {res.source === "local" ? "From your library" : "From your server while your library finishes syncing"} · {Math.round(res.tookMs)} ms
          </p>
          <SearchCatalogue query={q} filter={filter} localCount={localCount} />
        </>
      )}
    </div>
  );
}
