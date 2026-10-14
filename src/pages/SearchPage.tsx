import { MoreHorizontal, Pause, Play } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type MouseEvent, type ReactNode } from "react";
import { Art } from "../components/Art";
import { CardSkeleton } from "../components/Card";
import { openContextMenu } from "../components/ContextMenu";
import { shareItem } from "../components/ShareMenu";
import { Skeleton } from "../components/Skeleton";
import { FilterChips } from "../components/Tabs";
import { TrackList, TrackRow, TrackRowSkeleton, trackMenuItems } from "../components/TrackRow";
import { backend } from "../lib/backend";
import { modKey } from "../lib/platform";
import type { SearchFilter, SearchHit, SearchResults } from "../lib/types";
import type { MenuItem } from "../stores/ui";
import { player, usePlayer } from "../stores/player";
import { nav } from "../stores/router";
import { MAX_QUERY } from "../shell/SearchBox";
import { hitInfo, hitOf, mixedHits, openHit, playHit, rememberHit, type HitInfo } from "../shell/searchModel";
import { AlbumCard, ArtistCard, PlaylistCard, useContextState } from "./cards";
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
  }, [q, filter]);

  // Results for another chip would show the wrong layout: treat them as absent.
  const res = shown && shown.filter === filter ? shown.res : null;
  const songIds = useMemo(() => (res ? res.songs.map((s) => s.id) : []), [res]);
  const rows = useMemo(() => (res && filter === "all" ? mixedHits(res) : []), [res, filter]);

  if (!q)
    return (
      <div className="page-pad sr">
        <div className="sr-empty">
          <h1 className="section-title">Search your library</h1>
          <p>Find artists, albums, songs and playlists on your server. Use the search box at the top, or press {modKey}+L.</p>
        </div>
      </div>
    );

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
      <FilterChips label="Filter results" items={FILTERS} value={filter} onChange={setFilter} className="sr-chips" />
      {!res ? (
        failed ? (
          <div className="sr-empty">
            <h2 className="section-title">Search isn't available right now</h2>
            <p>Try again in a moment.</p>
          </div>
        ) : (
          <ResultsSkeleton filter={filter} />
        )
      ) : empty ? (
        <div className="sr-empty">
          <h2 className="section-title">No results for “{q.length > 60 ? `${q.slice(0, 60)}…` : q}”</h2>
          <p>Check the spelling, or try fewer or different keywords.</p>
        </div>
      ) : (
        <>
          {filter === "all" && (
            <div className="sr-all">
              {res.top && <TopResult hit={res.top} songIds={songIds} />}
              <ul className="sr-list" aria-label="Results">
                {rows.map((h) => (
                  <ResultRow key={hitInfo(h)!.key} hit={h} songIds={songIds} />
                ))}
              </ul>
            </div>
          )}
          {filter === "songs" && (
            <TrackList label="Songs">
              {res.songs.map((t, i) => (
                <TrackRow
                  key={t.id}
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
              ))}
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
        </>
      )}
    </div>
  );
}
