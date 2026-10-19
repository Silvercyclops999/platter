/**
 * An album on the phone: the cover at about 55% of the width, its title,
 * artist and year; download, more, shuffle and a large play;
 * then the songs, each with the downloaded mark before its artist line.
 */
import { MoreHorizontal, Pause, Play, Shuffle } from "lucide-react";
import type { CSSProperties } from "react";
import { Art } from "../../components/Art";
import { discGroups } from "../../pages/album";
import { DownloadButton } from "../../components/DownloadButton";
import { backend } from "../../lib/backend";
import { formatLongDuration, releaseTypeLabel } from "../../lib/format";
import { useLoad } from "../../lib/useLoad";
import { useArtColours } from "../../lib/useTint";
import { player, usePlayer } from "../../stores/player";
import { nav } from "../../stores/router";
import { phone } from "../phoneUi";
import { SleeveCard } from "../NowPlaying";
import { BackBar, Carousel, PageFailed, PageLoading, ShelfCard, SongRow, countLine } from "./common";

export function PhoneAlbum({ id }: { id: string }) {
  const { data, error, retry } = useLoad(() => backend.albumPage(id), [id]);
  const ctx = usePlayer((s) => s.state?.queue.context ?? null);
  const status = usePlayer((s) => s.state?.status ?? "idle");
  const colours = useArtColours(data?.album.tint, data?.album.thumb);
  if (error && !data) return <PageFailed what="this album" retry={retry} />;
  if (!data) return <PageLoading />;
  const { album, tracks } = data;
  const here = ctx?.kind === "album" && ctx.id === album.id;
  const playing = here && (status === "playing" || status === "loading" || status === "buffering");
  const facts = [releaseTypeLabel(album), album.year ? String(album.year) : null].filter(Boolean).join(" · ");
  return (
    <div className="ph-page ph-album" style={{ "--ph-tint": colours?.vivid ?? "var(--color-bg-overlay)" } as CSSProperties}>
      <BackBar title={album.title} after={320} />
      <header className="ph-album__head">
        <div className="ph-album__cover">
          <Art path={album.thumb} fluid requestSize={480} sleeve={{ title: album.title, artist: album.artistTitle }} />
        </div>
        <h1 className="ph-album__title">{album.title}</h1>
        <button type="button" className="ph-album__artist" onClick={() => nav.go({ name: "artist", id: album.artistId })}>
          {album.artistTitle}
        </button>
        <p className="ph-album__facts">
          <span className="tabular">{facts}</span>
        </p>
      </header>
      <div className="ph-actions">
        <div className="ph-actions__left">
          <DownloadButton kind="album" id={album.id} title={album.title} size="md" />
          <button type="button" className="ph-icon-btn" aria-label={`More for ${album.title}`} onClick={() => phone.push({ kind: "album", album })}>
            <MoreHorizontal size={24} aria-hidden="true" />
          </button>
        </div>
        <div className="ph-actions__right">
          <button type="button" className="ph-icon-btn" aria-label={`Shuffle ${album.title}`} onClick={() => void player.playAlbum(album.id, undefined, true)}>
            <Shuffle size={24} aria-hidden="true" />
          </button>
          <button
            type="button"
            className="ph-play ph-play--page"
            aria-label={playing ? "Pause" : `Play ${album.title}`}
            onClick={() => (here ? void player.toggle() : void player.playAlbum(album.id))}
          >
            {playing ? <Pause size={28} fill="currentColor" aria-hidden="true" /> : <Play size={28} fill="currentColor" aria-hidden="true" />}
          </button>
        </div>
      </div>
      {discGroups(tracks).map((g) => (
        <section key={g.disc ?? "all"} className="ph-disc-group" aria-label={g.disc !== null ? `Disc ${g.disc}` : undefined}>
          {g.disc !== null && (
            <h2 className="ph-disc-head">
              <span>Disc {g.disc}</span>
              <span className="ph-disc-head__count tabular">{countLine(g.tracks.length, "song")}</span>
            </h2>
          )}
          <ul className="ph-songs" aria-label={g.disc !== null ? `Disc ${g.disc} songs` : "Songs"}>
            {g.tracks.map((t) => (
              <SongRow key={t.id} track={t} index={t.trackNumber} art={false} onPlay={() => void player.playAlbum(album.id, t.id)} />
            ))}
          </ul>
        </section>
      ))}
      <p className="ph-foot tabular">
        {countLine(tracks.length, "song")}, {formatLongDuration(data.totalDurationMs)}
      </p>
      <SleeveCard track={{ albumId: album.id, albumTitle: album.title, artistTitle: album.artistTitle, thumb: album.thumb }} link={false} />
      {data.moreByArtist.length > 0 && (
        <Carousel title={`More by ${album.artistTitle}`}>
          {data.moreByArtist.slice(0, 12).map((a) => (
            <ShelfCard key={a.id} title={a.title} sub={a.year ? String(a.year) : undefined} artist={a.artistTitle} thumb={a.thumb} kind="album" onOpen={() => nav.go({ name: "album", id: a.id })} />
          ))}
        </Carousel>
      )}
    </div>
  );
}
