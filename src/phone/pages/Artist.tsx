/**
 * An artist on the phone: a large photo with the name over it, which the
 * page slides up over as it scrolls; what of theirs is in your library; top
 * tracks ranked by your own plays; then the discography as shelves.
 */
import { Pause, Play, Shuffle } from "lucide-react";
import { useState, type UIEvent } from "react";
import { Art } from "../../components/Art";
import { backend } from "../../lib/backend";
import { SECTION_LABELS, plural } from "../../lib/format";
import type { Track } from "../../lib/types";
import { useLoad } from "../../lib/useLoad";
import { photoPath } from "../../pages/artist";
import { player, usePlayer } from "../../stores/player";
import { nav } from "../../stores/router";
import { BackBar, Carousel, PageFailed, PageLoading, ShelfCard, SongRow } from "./common";

/** Top tracks shown. */
export const TOP_TRACKS = 5;

/** The artist's songs most played here, most first (ties keep the server's order). */
export function topByPlays(tracks: Track[], n = TOP_TRACKS): Track[] {
  return tracks
    .map((t, i) => ({ t, i }))
    .sort((a, b) => b.t.viewCount - a.t.viewCount || a.i - b.i)
    .slice(0, n)
    .map((x) => x.t);
}

/** "In your library: 4 albums, 52 songs". */
export function inLibraryLine(albums: number, tracks: number): string {
  return `In your library: ${plural(albums, "album")}, ${plural(tracks, "song")}`;
}

export function PhoneArtist({ id }: { id: string }) {
  const { data, error, retry } = useLoad(() => backend.artistPage(id), [id]);
  const ctx = usePlayer((s) => s.state?.queue.context ?? null);
  const status = usePlayer((s) => s.state?.status ?? "idle");
  const [scrolled, setScrolled] = useState(0);
  if (error && !data) return <PageFailed what="this artist" retry={retry} />;
  if (!data) return <PageLoading />;
  const { artist, sections } = data;
  const albums = sections.flatMap((s) => s.albums);
  const songs = albums.reduce((n, a) => n + a.trackCount, 0);
  const top = topByPlays(data.popular);
  const here = ctx?.kind === "artist" && ctx.id === artist.id;
  const playing = here && (status === "playing" || status === "loading" || status === "buffering");
  const onScroll = (e: UIEvent<HTMLDivElement>) => setScrolled(e.currentTarget.scrollTop);
  // The photo fades and holds still while the page slides over it.
  const fade = Math.max(0, 1 - scrolled / 260);
  return (
    <div className="ph-page ph-artist" onScroll={onScroll} data-testid="artist-page">
      <div className="ph-artist__hero" style={{ opacity: fade }}>
        <Art path={photoPath(artist)} fluid requestSize={600} kind="artist" monogram={artist.title} />
      </div>
      <BackBar title={artist.title} shown={scrolled > 220} />
      <h1 className="ph-artist__name">{artist.title}</h1>
      <div className="ph-artist__body">
        <div className="ph-actions">
          <div className="ph-actions__left">
            <span className="ph-artist__lib tabular">{inLibraryLine(albums.length, songs)}</span>
          </div>
          <div className="ph-actions__right">
            <button type="button" className="ph-icon-btn" aria-label={`Shuffle ${artist.title}`} onClick={() => void player.playArtist(artist.id, undefined, true)}>
              <Shuffle size={24} aria-hidden="true" />
            </button>
            <button
              type="button"
              className="ph-play ph-play--page"
              aria-label={playing ? "Pause" : `Play ${artist.title}`}
              onClick={() => (here ? void player.toggle() : void player.playArtist(artist.id))}
            >
              {playing ? <Pause size={28} fill="currentColor" aria-hidden="true" /> : <Play size={28} fill="currentColor" aria-hidden="true" />}
            </button>
          </div>
        </div>
        {top.length > 0 && (
          <section aria-label="Top tracks">
            <h2 className="ph-section-title">Top tracks</h2>
            <p className="ph-section-sub">By your plays</p>
            <ul className="ph-songs">
              {top.map((t, i) => (
                <SongRow
                  key={t.id}
                  track={t}
                  index={i + 1}
                  sub={t.viewCount ? plural(t.viewCount, "play") : t.albumTitle}
                  onPlay={() => void player.playTracks(top.map((x) => x.id), t.id)}
                />
              ))}
            </ul>
          </section>
        )}
        {sections.map((s) => (
          <Carousel key={`${s.section}:${s.custom ?? ""}`} title={s.custom ?? SECTION_LABELS[s.section]}>
            {s.albums.map((a) => (
              <ShelfCard key={a.id} title={a.title} sub={a.year ? String(a.year) : undefined} artist={a.artistTitle} thumb={a.thumb} kind="album" onOpen={() => nav.go({ name: "album", id: a.id })} />
            ))}
          </Carousel>
        ))}
      </div>
    </div>
  );
}
