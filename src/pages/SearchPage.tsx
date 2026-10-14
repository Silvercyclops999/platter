import { Clock3, Play } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { Art } from "../components/Art";
import { Shelf, ShelfItem } from "../components/Shelf";
import { Skeleton } from "../components/Skeleton";
import { FilterChips } from "../components/Tabs";
import { TrackList, TrackRow, TrackRowSkeleton } from "../components/TrackRow";
import { backend } from "../lib/backend";
import { trackArtistLine } from "../lib/format";
import type { Album, Artist, Playlist, SearchFilter, SearchHit, SearchResults, Track } from "../lib/types";
import { player, usePlayer } from "../stores/player";
import { nav, useRouter } from "../stores/router";
import { toast } from "../stores/toasts";
import { AlbumCard, ArtistCard, PlaylistCard } from "./cards";

/** Longest query sent to the core; anything past it is ignored. */
export const MAX_QUERY = 200;
/** Songs shown next to the top result on the All tab. */
const ALL_SONGS = 4;

const FILTERS: { id: SearchFilter; label: string }[] = [
  { id: "all", label: "All" },
  { id: "songs", label: "Songs" },
  { id: "artists", label: "Artists" },
  { id: "albums", label: "Albums" },
  { id: "playlists", label: "Playlists" },
];

// ---------------------------------------------------------------- selection + actions

/** One keyboard-selectable result, in on-screen order. */
export type Pick =
  | { kind: "top"; hit: SearchHit }
  | { kind: "song"; track: Track }
  | { kind: "artist"; artist: Artist }
  | { kind: "album"; album: Album }
  | { kind: "playlist"; playlist: Playlist };

/** Results in the order arrows walk them: top result, songs, then shelves. */
export function picksFor(res: SearchResults, filter: SearchFilter): Pick[] {
  const out: Pick[] = [];
  const all = filter === "all";
  if (all && res.top) out.push({ kind: "top", hit: res.top });
  if (all || filter === "songs") visibleSongs(res, filter).forEach((track) => out.push({ kind: "song", track }));
  if (all || filter === "artists") res.artists.forEach((artist) => out.push({ kind: "artist", artist }));
  if (all || filter === "albums") res.albums.forEach((album) => out.push({ kind: "album", album }));
  if (all || filter === "playlists") res.playlists.forEach((playlist) => out.push({ kind: "playlist", playlist }));
  return out;
}

export function visibleSongs(res: SearchResults, filter: SearchFilter): Track[] {
  return filter === "all" ? res.songs.slice(0, ALL_SONGS) : filter === "songs" ? res.songs : [];
}

/** Normalise a pick to the item it stands for. */
function target(p: Pick): Pick {
  if (p.kind !== "top") return p;
  const h = p.hit;
  if (h.kind === "artist" && h.artist) return { kind: "artist", artist: h.artist };
  if (h.kind === "album" && h.album) return { kind: "album", album: h.album };
  if (h.kind === "track" && h.track) return { kind: "song", track: h.track };
  if (h.kind === "playlist" && h.playlist) return { kind: "playlist", playlist: h.playlist };
  return p;
}

/** Play a result. Songs play in a "search" context of the songs on screen. */
export function playPick(p: Pick, songs: Track[]) {
  const t = target(p);
  switch (t.kind) {
    case "song": {
      const ids = songs.map((s) => s.id);
      return player.playTracks(ids.includes(t.track.id) ? ids : [t.track.id], t.track.id, "search");
    }
    case "artist":
      return player.playArtist(t.artist.id);
    case "album":
      return player.playAlbum(t.album.id);
    case "playlist":
      return player.playPlaylist(t.playlist.id);
  }
}

/** Add a result to the end of the queue (a whole album, playlist or the artist's top songs). */
export async function queuePick(p: Pick) {
  const t = target(p);
  try {
    let ids: string[] = [];
    let what = "";
    switch (t.kind) {
      case "song":
        ids = [t.track.id];
        what = t.track.title;
        break;
      case "album":
        ids = (await backend.albumPage(t.album.id)).tracks.map((x) => x.id);
        what = t.album.title;
        break;
      case "playlist":
        ids = (await backend.playlistPage(t.playlist.id)).tracks.map((x) => x.id);
        what = t.playlist.title;
        break;
      case "artist":
        ids = (await backend.artistPage(t.artist.id)).popular.map((x) => x.id);
        what = `Popular songs by ${t.artist.title}`;
        break;
    }
    if (!ids.length) return;
    await backend.queueAdd(ids, "end");
    toast.info(`Added to queue: ${what}`);
  } catch (e) {
    const msg = e && typeof e === "object" && "message" in e ? String((e as { message: unknown }).message) : "";
    toast.error(msg || "Couldn't add to queue");
  }
}

function pickLabel(p: Pick): string {
  const t = target(p);
  switch (t.kind) {
    case "song":
      return `Song, ${t.track.title} by ${trackArtistLine(t.track)}`;
    case "artist":
      return `Artist, ${t.artist.title}`;
    case "album":
      return `Album, ${t.album.title} by ${t.album.artistTitle}`;
    case "playlist":
      return `Playlist, ${t.playlist.title}`;
    default:
      return "";
  }
}

/** True for the header search box (the only place result arrows apply). */
function isSearchBox(el: EventTarget | null): boolean {
  return el instanceof HTMLInputElement && !!el.closest(".main-header");
}

// ---------------------------------------------------------------- pieces

function TopResult({ hit, selected, onPlay }: { hit: SearchHit; selected: boolean; onPlay: () => void }) {
  let title = "";
  let sub = "";
  let thumb: string | null | undefined;
  let open = () => {};
  let round = false;
  if (hit.kind === "artist" && hit.artist) {
    const a = hit.artist;
    title = a.title;
    sub = "Artist";
    thumb = a.thumb;
    round = true;
    open = () => nav.go({ name: "artist", id: a.id });
  } else if (hit.kind === "album" && hit.album) {
    const a = hit.album;
    title = a.title;
    sub = `Album · ${a.artistTitle}`;
    thumb = a.thumb;
    open = () => nav.go({ name: "album", id: a.id });
  } else if (hit.kind === "track" && hit.track) {
    const t = hit.track;
    title = t.title;
    sub = `Song · ${trackArtistLine(t)}`;
    thumb = t.thumb;
    open = () => nav.go({ name: "album", id: t.albumId });
  } else if (hit.kind === "playlist" && hit.playlist) {
    const p = hit.playlist;
    title = p.title;
    sub = "Playlist";
    thumb = p.thumb;
    open = () => nav.go({ name: "playlist", id: p.id });
  }
  return (
    <div className={`top-result card search-pick ${selected ? "is-selected" : ""}`}>
      <Art path={thumb} size={96} round={round} kind={hit.kind === "artist" ? "artist" : "album"} />
      <button type="button" className="card__link top-result__title" onClick={open}>
        <span className="truncate">{title}</span>
      </button>
      <div className="top-result__sub truncate">{sub}</div>
      <button
        type="button"
        className="card__play"
        aria-label={`Play ${title}`}
        onClick={onPlay}
      >
        <Play size={20} fill="currentColor" aria-hidden="true" />
      </button>
    </div>
  );
}

function SectionHead({ id, title, onShowAll }: { id: string; title: string; onShowAll?: () => void }) {
  return (
    <div className="search__section-head">
      <h2 id={id} className="section-title">
        {title}
      </h2>
      {onShowAll && (
        <button type="button" className="shelf__all" onClick={onShowAll}>
          Show all
        </button>
      )}
    </div>
  );
}

function Recent() {
  const [recent, setRecent] = useState<string[] | null>(null);
  useEffect(() => {
    let alive = true;
    backend.recentSearches().then(
      (r) => alive && setRecent(r),
      () => alive && setRecent([]),
    );
    return () => {
      alive = false;
    };
  }, []);
  if (recent === null)
    return (
      <div className="recent" aria-hidden="true">
        <Skeleton width={200} height={24} style={{ marginBottom: 12 }} />
        <div className="recent__list">
          {[96, 132, 112].map((w, i) => (
            <Skeleton key={i} width={w} height={36} radius="pill" />
          ))}
        </div>
      </div>
    );
  if (!recent.length)
    return (
      <div className="search-empty">
        <h2 className="section-title">Search your library</h2>
        <p>Find artists, albums, songs and playlists on your server.</p>
      </div>
    );
  return (
    <section className="recent" aria-labelledby="recent-h">
      <div className="recent__head">
        <h2 id="recent-h" className="section-title">
          Recent searches
        </h2>
        <button
          type="button"
          className="shelf__all"
          onClick={() =>
            backend.recentSearchesClear().then(
              () => setRecent([]),
              () => toast.error("Couldn't clear recent searches"),
            )
          }
        >
          Clear recent searches
        </button>
      </div>
      <ul className="recent__list">
        {recent.map((q) => (
          <li key={q}>
            <button type="button" className="recent__item" onClick={() => useRouter.getState().replace({ name: "search", query: q })}>
              <Clock3 size={16} aria-hidden="true" />
              <span className="truncate">{q}</span>
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}

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
    return <Shelf title={FILTERS.find((f) => f.id === filter)!.label} loading skeletonKind={filter === "artists" ? "artist" : filter === "playlists" ? "playlist" : "album"} />;
  return (
    <div className="search__grid" aria-hidden="true">
      <div>
        <Skeleton width={140} height={24} style={{ marginBottom: 12 }} />
        <Skeleton height={220} radius="card" />
      </div>
      <div>
        <Skeleton width={90} height={24} style={{ marginBottom: 12 }} />
        <TrackList label="Loading songs" header={false}>
          {Array.from({ length: ALL_SONGS }, (_, i) => (
            <TrackRowSkeleton key={i} showArt showAlbum={false} />
          ))}
        </TrackList>
      </div>
    </div>
  );
}

function PickCell({ selected, children }: { selected: boolean; children: ReactNode }) {
  return (
    <div className={`search-pick ${selected ? "is-selected" : ""}`}>
      {children}
    </div>
  );
}

// ---------------------------------------------------------------- page

interface Shown {
  query: string;
  filter: SearchFilter;
  res: SearchResults;
}

export function SearchPage({ query }: { query: string }) {
  const [filter, setFilter] = useState<SearchFilter>("all");
  const [shown, setShown] = useState<Shown | null>(null);
  const [loading, setLoading] = useState(false);
  const [failed, setFailed] = useState(false);
  const [sel, setSel] = useState(-1);
  const seq = useRef(0);
  const root = useRef<HTMLDivElement>(null);
  const nowTrackId = usePlayer((s) => s.state?.track?.id);
  const status = usePlayer((s) => s.state?.status);
  const q = query.trim().slice(0, MAX_QUERY);

  // Search on every keystroke. Each request gets a sequence number and only
  // the newest one may update the screen, so a slow answer to an older query
  // can never replace newer results.
  useEffect(() => {
    const mine = ++seq.current;
    setSel(-1);
    if (!q) {
      setShown(null);
      setLoading(false);
      setFailed(false);
      return;
    }
    setLoading(true);
    backend.search({ query: q, filter, limit: filter === "all" ? 10 : 50 }).then(
      (res) => {
        if (mine !== seq.current) return;
        setShown({ query: q, filter, res });
        setLoading(false);
        setFailed(false);
      },
      () => {
        if (mine !== seq.current) return;
        setLoading(false);
        setFailed(true);
      },
    );
  }, [q, filter]);

  // Results for another tab would show the wrong layout: treat them as absent.
  const res = shown && shown.filter === filter ? shown.res : null;
  const picks = useMemo(() => (res ? picksFor(res, filter) : []), [res, filter]);
  const songs = useMemo(() => (res ? visibleSongs(res, filter) : []), [res, filter]);

  // Keyboard: arrows move through results while typing; Enter plays,
  // Shift+Enter adds to the queue.
  const live = useRef({ picks, songs, sel, q });
  live.current = { picks, songs, sel, q };
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!isSearchBox(e.target) || e.altKey || e.ctrlKey || e.metaKey) return;
      const { picks, songs, sel, q } = live.current;
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        if (!picks.length) return;
        e.preventDefault();
        const next = e.key === "ArrowDown" ? Math.min(sel + 1, picks.length - 1) : Math.max(sel - 1, -1);
        setSel(next);
      } else if (e.key === "Enter" && sel >= 0 && picks[sel]) {
        e.preventDefault();
        if (q) backend.recentSearchAdd(q).catch(() => {});
        if (e.shiftKey) queuePick(picks[sel]);
        else playPick(picks[sel], songs);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // Keep the selection on screen.
  useEffect(() => {
    if (sel < 0) return;
    const el = root.current?.querySelector<HTMLElement>(".search-pick.is-selected, .track-row.is-selected");
    el?.scrollIntoView?.({ block: "nearest", inline: "nearest" });
  }, [sel]);

  if (!q)
    return (
      <div className="page-pad">
        <Recent />
      </div>
    );

  const empty = res && !picks.length;
  let n = filter === "all" && res?.top ? 1 : 0;
  const songBase = n;
  if (res) n += songs.length;
  const play = (p: Pick) => {
    backend.recentSearchAdd(q).catch(() => {});
    playPick(p, songs);
  };
  const shelfPicks = <T,>(items: T[]) => {
    const base = n;
    n += items.length;
    return base;
  };

  return (
    <div className="page-pad search" ref={root} aria-busy={loading}>
      <FilterChips label="Filter results" items={FILTERS} value={filter} onChange={setFilter} className="search__chips" />
      {!res ? (
        failed ? (
          <div className="search-empty">
            <h2 className="section-title">Search isn't available right now</h2>
            <p>Try again in a moment.</p>
          </div>
        ) : (
          <ResultsSkeleton filter={filter} />
        )
      ) : empty ? (
        <div className="search-empty">
          <h2 className="section-title">No results for “{q.length > 60 ? `${q.slice(0, 60)}…` : q}”</h2>
          <p>Check the spelling, or try fewer or different keywords.</p>
        </div>
      ) : (
        <>
          {filter === "all" && (res.top || songs.length > 0) && (
            <div className="search__grid">
              {res.top && (
                <section aria-labelledby="top-h">
                  <SectionHead id="top-h" title="Top result" />
                  <TopResult hit={res.top} selected={sel === 0} onPlay={() => play({ kind: "top", hit: res.top! })} />
                </section>
              )}
              {songs.length > 0 && (
                <section aria-labelledby="songs-h" className="search__songs">
                  <SectionHead id="songs-h" title="Songs" onShowAll={res.songs.length >= ALL_SONGS ? () => setFilter("songs") : undefined} />
                  <TrackList label="Songs" header={false}>
                    {songs.map((t, i) => (
                      <TrackRow
                        key={t.id}
                        track={t}
                        index={i + 1}
                        compact
                        selected={sel === songBase + i}
                        active={t.id === nowTrackId}
                        playing={t.id === nowTrackId && status === "playing"}
                        onPlay={() => play({ kind: "song", track: t })}
                      />
                    ))}
                  </TrackList>
                </section>
              )}
            </div>
          )}
          {filter === "songs" && (
            <TrackList label="Songs">
              {songs.map((t, i) => (
                <TrackRow
                  key={t.id}
                  track={t}
                  index={i + 1}
                  showArt
                  selected={sel === songBase + i}
                  active={t.id === nowTrackId}
                  playing={t.id === nowTrackId && status === "playing"}
                  onPlay={() => play({ kind: "song", track: t })}
                />
              ))}
            </TrackList>
          )}
          {(filter === "all" || filter === "artists") && res.artists.length > 0 && (
            <Shelf title="Artists" onShowAll={filter === "all" ? () => setFilter("artists") : undefined}>
              {(() => {
                const base = shelfPicks(res.artists);
                return res.artists.map((a, i) => (
                  <ShelfItem key={a.id}>
                    <PickCell selected={sel === base + i}>
                      <ArtistCard artist={a} />
                    </PickCell>
                  </ShelfItem>
                ));
              })()}
            </Shelf>
          )}
          {(filter === "all" || filter === "albums") && res.albums.length > 0 && (
            <Shelf title="Albums" onShowAll={filter === "all" ? () => setFilter("albums") : undefined}>
              {(() => {
                const base = shelfPicks(res.albums);
                return res.albums.map((a, i) => (
                  <ShelfItem key={a.id}>
                    <PickCell selected={sel === base + i}>
                      <AlbumCard album={a} />
                    </PickCell>
                  </ShelfItem>
                ));
              })()}
            </Shelf>
          )}
          {(filter === "all" || filter === "playlists") && res.playlists.length > 0 && (
            <Shelf title="Playlists" onShowAll={filter === "all" ? () => setFilter("playlists") : undefined}>
              {(() => {
                const base = shelfPicks(res.playlists);
                return res.playlists.map((p, i) => (
                  <ShelfItem key={p.id}>
                    <PickCell selected={sel === base + i}>
                      <PlaylistCard playlist={p} />
                    </PickCell>
                  </ShelfItem>
                ));
              })()}
            </Shelf>
          )}
          <p className="search__meta">
            {res.source === "local" ? "From your library" : "From your server while your library finishes syncing"} ·{" "}
            {Math.round(res.tookMs)} ms
          </p>
        </>
      )}
      <div className="sr-only" aria-live="polite">
        {sel >= 0 && picks[sel] ? `${pickLabel(picks[sel])}. Enter to play, Shift+Enter to add to queue.` : ""}
      </div>
    </div>
  );
}
