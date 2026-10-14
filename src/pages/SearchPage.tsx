import { Clock3, Play } from "lucide-react";
import { useEffect, useState } from "react";
import { Art } from "../components/Art";
import { Shelf, ShelfItem } from "../components/Shelf";
import { Skeleton } from "../components/Skeleton";
import { FilterChips } from "../components/Tabs";
import { TrackList, TrackRow, TrackRowSkeleton } from "../components/TrackRow";
import { backend } from "../lib/backend";
import { trackArtistLine } from "../lib/format";
import type { SearchFilter, SearchHit, SearchResults } from "../lib/types";
import { player, usePlayer } from "../stores/player";
import { nav, useRouter } from "../stores/router";
import { AlbumCard, ArtistCard, PlaylistCard } from "./cards";

const FILTERS: { id: SearchFilter; label: string }[] = [
  { id: "all", label: "All" },
  { id: "songs", label: "Songs" },
  { id: "artists", label: "Artists" },
  { id: "albums", label: "Albums" },
  { id: "playlists", label: "Playlists" },
];

function TopResult({ hit }: { hit: SearchHit }) {
  let title = "";
  let sub = "";
  let thumb: string | null | undefined;
  let open = () => {};
  let play = () => {};
  let round = false;
  if (hit.kind === "artist" && hit.artist) {
    const a = hit.artist;
    title = a.title;
    sub = "Artist";
    thumb = a.thumb;
    round = true;
    open = () => nav.go({ name: "artist", id: a.id });
    play = () => player.playArtist(a.id);
  } else if (hit.kind === "album" && hit.album) {
    const a = hit.album;
    title = a.title;
    sub = `Album · ${a.artistTitle}`;
    thumb = a.thumb;
    open = () => nav.go({ name: "album", id: a.id });
    play = () => player.playAlbum(a.id);
  } else if (hit.kind === "track" && hit.track) {
    const t = hit.track;
    title = t.title;
    sub = `Song · ${trackArtistLine(t)}`;
    thumb = t.thumb;
    open = () => nav.go({ name: "album", id: t.albumId });
    play = () => player.playTracks([t.id], t.id, "search");
  } else if (hit.kind === "playlist" && hit.playlist) {
    const p = hit.playlist;
    title = p.title;
    sub = "Playlist";
    thumb = p.thumb;
    open = () => nav.go({ name: "playlist", id: p.id });
    play = () => player.playPlaylist(p.id);
  }
  return (
    <div className="top-result card">
      <Art path={thumb} size={96} round={round} kind={hit.kind === "artist" ? "artist" : "album"} />
      <button type="button" className="card__link top-result__title" onClick={open}>
        <span className="truncate">{title}</span>
      </button>
      <div className="top-result__sub truncate">{sub}</div>
      <button type="button" className="card__play" aria-label={`Play ${title}`} onClick={play}>
        <Play size={20} fill="currentColor" aria-hidden="true" />
      </button>
    </div>
  );
}

function Recent() {
  const [recent, setRecent] = useState<string[] | null>(null);
  useEffect(() => {
    backend.recentSearches().then(setRecent, () => setRecent([]));
  }, []);
  if (recent === null)
    return (
      <div className="recent">
        <Skeleton width={160} height={20} />
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
          onClick={() => backend.recentSearchesClear().then(() => setRecent([]))}
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

export function SearchPage({ query }: { query: string }) {
  const [filter, setFilter] = useState<SearchFilter>("all");
  const [res, setRes] = useState<SearchResults | null>(null);
  const [loading, setLoading] = useState(false);
  const nowTrackId = usePlayer((s) => s.state?.track?.id);
  const status = usePlayer((s) => s.state?.status);
  const q = query.trim();

  useEffect(() => {
    if (!q) {
      setRes(null);
      return;
    }
    let alive = true;
    setLoading(true);
    // Keep the previous results on screen while typing (no flashing).
    backend.search({ query: q, filter, limit: filter === "all" ? 10 : 50 }).then(
      (r) => {
        if (!alive) return;
        setRes(r);
        setLoading(false);
      },
      () => alive && setLoading(false),
    );
    return () => {
      alive = false;
    };
  }, [q, filter]);

  if (!q) return <div className="page-pad"><Recent /></div>;

  const empty = res && !res.top && !res.songs.length && !res.artists.length && !res.albums.length && !res.playlists.length;
  const playSong = (id: string) => res && player.playTracks(res.songs.map((s) => s.id), id, "search");

  return (
    <div className="page-pad search">
      <FilterChips label="Filter results" items={FILTERS} value={filter} onChange={setFilter} className="search__chips" />
      {!res && loading ? (
        <div className="search__grid">
          <Skeleton height={220} radius="card" />
          <TrackList label="Songs" header={false}>
            {Array.from({ length: 4 }, (_, i) => (
              <TrackRowSkeleton key={i} showArt showAlbum={false} />
            ))}
          </TrackList>
        </div>
      ) : empty ? (
        <div className="search-empty">
          <h2 className="section-title">No results for “{q}”</h2>
          <p>Check the spelling, or try fewer or different keywords.</p>
        </div>
      ) : res ? (
        <>
          {filter === "all" && (
            <div className="search__grid">
              {res.top && (
                <section aria-labelledby="top-h">
                  <h2 id="top-h" className="section-title">
                    Top result
                  </h2>
                  <TopResult hit={res.top} />
                </section>
              )}
              {res.songs.length > 0 && (
                <section aria-labelledby="songs-h" className="search__songs">
                  <h2 id="songs-h" className="section-title">
                    Songs
                  </h2>
                  <TrackList label="Songs" header={false}>
                    {res.songs.slice(0, 4).map((t, i) => (
                      <TrackRow
                        key={t.id}
                        track={t}
                        index={i + 1}
                        compact
                        active={t.id === nowTrackId}
                        playing={t.id === nowTrackId && status === "playing"}
                        onPlay={() => playSong(t.id)}
                      />
                    ))}
                  </TrackList>
                </section>
              )}
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
                  onPlay={() => playSong(t.id)}
                />
              ))}
            </TrackList>
          )}
          {(filter === "all" || filter === "artists") && res.artists.length > 0 && (
            <Shelf title="Artists" onShowAll={filter === "all" ? () => setFilter("artists") : undefined}>
              {res.artists.map((a) => (
                <ShelfItem key={a.id}>
                  <ArtistCard artist={a} />
                </ShelfItem>
              ))}
            </Shelf>
          )}
          {(filter === "all" || filter === "albums") && res.albums.length > 0 && (
            <Shelf title="Albums" onShowAll={filter === "all" ? () => setFilter("albums") : undefined}>
              {res.albums.map((a) => (
                <ShelfItem key={a.id}>
                  <AlbumCard album={a} />
                </ShelfItem>
              ))}
            </Shelf>
          )}
          {(filter === "all" || filter === "playlists") && res.playlists.length > 0 && (
            <Shelf title="Playlists">
              {res.playlists.map((p) => (
                <ShelfItem key={p.id}>
                  <PlaylistCard playlist={p} />
                </ShelfItem>
              ))}
            </Shelf>
          )}
          <p className="search__meta" aria-live="polite">
            {res.source === "local" ? "From your library cache" : "From the server"} · {res.tookMs} ms
            {loading && <span className="sr-only"> Updating…</span>}
          </p>
        </>
      ) : null}
    </div>
  );
}
