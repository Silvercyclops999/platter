/**
 * An album on the phone: the cover at about 55% of the width, its title,
 * artist and year; download, more, shuffle and a large play;
 * then the songs, each with the downloaded mark before its artist line.
 */
import { ChevronDown, MoreHorizontal, Pause, Play, Shuffle } from "lucide-react";
import { useState, type CSSProperties } from "react";
import { Art } from "../../components/Art";
import { discGroups } from "../../pages/album";
import { PhoneDownload } from "../PhoneDownload";
import { backend } from "../../lib/backend";
import { formatLongDuration, releaseTypeLabel } from "../../lib/format";
import { useLoad } from "../../lib/useLoad";
import { useArtColours } from "../../lib/useTint";
import { player, usePlayer } from "../../stores/player";
import { nav } from "../../stores/router";
import { phone } from "../phoneUi";
import { SleeveCard } from "../NowPlaying";
import { BackBar, PageFailed, PageLoading, ShelfCard, SongRow, countLine } from "./common";
import { UpgradeCard, useAlbumUpgrade } from "../../pages/LidarrUpgrade";
import { useSleeveNotes } from "../../lib/useSleeveNotes";
import { RoleLines, SongBySong, songsWithCredits } from "../../pages/SleeveNotes";
import { tap } from "../bits";



/**
 * The record's people, by role, set like the computer's sleeve notes
 * ("Produced by" and the names beside it); each opens their credit page.
 */
function AlbumPeople({ albumId }: { albumId: string }) {
  const { data } = useSleeveNotes(albumId);
  const [open, setOpen] = useState(false);
  const groups = data?.groups.filter((g) => g.people.length) ?? [];
  const songs = data ? songsWithCredits(data) : [];
  if (!groups.length && !songs.length) return null;
  // A long Song by song folds behind its own row; a short one just shows.
  const folds = songs.length > SONG_BY_SONG_OPEN_UP_TO;
  const shown = !folds || open;
  return (
    <section className="sleeve__credits ph-sleeve-credits" aria-labelledby="ph-album-credits">
      <h2 id="ph-album-credits" className="sleeve__heading">
        Credits
      </h2>
      {groups.length > 0 ? <RoleLines groups={groups} marks={false} /> : <p className="sleeve-muted">No album credits yet.</p>}
      {songs.length > 0 && (
        <>
          {folds ? (
            <button
              type="button"
              className="ph-sbs-toggle"
              aria-expanded={open}
              onClick={() => {
                tap("selection");
                setOpen((o) => !o);
              }}
            >
              <span className="sleeve__subheading">Song by song</span>
              <span className="ph-sbs-toggle__count tabular">{countLine(songs.length, "song")}</span>
              <ChevronDown size={18} aria-hidden="true" className="ph-sbs-toggle__chev" />
            </button>
          ) : (
            <h3 className="sleeve__subheading">Song by song</h3>
          )}
          {shown && data && <SongBySong n={data} />}
        </>
      )}
    </section>
  );
}

/** Song by song shows open up to this many songs; past it, it folds behind a row. */
export const SONG_BY_SONG_OPEN_UP_TO = 3;

export function PhoneAlbum({ id }: { id: string }) {
  const { data, error, retry } = useLoad(() => backend.albumPage(id), [id]);
  const ctx = usePlayer((s) => s.state?.queue.context ?? null);
  const status = usePlayer((s) => s.state?.status ?? "idle");
  const colours = useArtColours(data?.album.tint, data?.album.thumb);
  // Lidarr: a better copy of this record, offered when one is known.
  const upgrade = useAlbumUpgrade(data ? id : null);
  if (error && !data) return <PageFailed what="this album" retry={retry} framed />;
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
          <Art path={album.thumb} fluid requestSize={480} tint={album.tint} sleeve={{ title: album.title, artist: album.artistTitle }} />
        </div>
        <h1 className="ph-album__title">{album.title}</h1>
        <button type="button" className="ph-album__artist" onClick={() => nav.go({ name: "artist", id: album.artistId })}>
          {album.artistTitle}
        </button>
        <p className="ph-album__facts">
          <span className="tabular">{facts}</span>
        </p>
      </header>
      {data.archive?.era && (
        <button type="button" className="ph-album__era" onClick={() => nav.go({ name: "era", id: album.artistId, era: data.archive!.era! })}>
          From the {data.archive.era} era
        </button>
      )}
      <div className="ph-actions">
        <div className="ph-actions__left">
          <PhoneDownload kind="album" id={album.id} title={album.title} trackIds={() => tracks.map((t) => t.id)} />
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
      <div className="ph-get ph-album__upgrade">
        <UpgradeCard upgrade={upgrade} onChanged={upgrade.reload} />
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
      <AlbumPeople albumId={album.id} />
      {data.moreByArtist.length > 0 && (
        <section className="ph-shelf" aria-label={`More by ${album.artistTitle}`}>
          <div className="ph-disco__head">
            <h2 className="ph-shelf__title">More by {album.artistTitle}</h2>
            <button type="button" className="ph-link-btn" onClick={() => nav.go({ name: "discography", id: album.artistId })}>
              See discography
            </button>
          </div>
          <div className="ph-shelf__row">
            {data.moreByArtist.slice(0, 12).map((a) => (
              <ShelfCard key={a.id} title={a.title} sub={a.year ? String(a.year) : undefined} artist={a.artistTitle} thumb={a.thumb} tint={a.tint} kind="album" onOpen={() => nav.go({ name: "album", id: a.id })} />
            ))}
          </div>
        </section>
      )}
    </div>
  );
}
