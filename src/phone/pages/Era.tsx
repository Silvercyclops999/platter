/**
 * One era of an artist on the phone: its cover and dates, play and
 * shuffle, the era in a sentence with its releases on a timeline as on the
 * computer, then every song of it by record or build, newest build first.
 */
import { useEffect } from "react";
import { Art } from "../../components/Art";
import { backend } from "../../lib/backend";
import { plural, releaseTypeLabel } from "../../lib/format";
import { useLoad, useReload } from "../../lib/useLoad";
import { eraRange, formatArchiveDate } from "../../pages/era";
import { EraStory } from "../../pages/EraStory";
import { player, usePlayer } from "../../stores/player";
import { nav } from "../../stores/router";
import { PageFailed, PageLoading, SongRow } from "./common";
import { Hero, PageFrame, PlayRow } from "./kit";

function shuffled<T>(list: T[]): T[] {
  const out = [...list];
  for (let i = out.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [out[i], out[j]] = [out[j], out[i]];
  }
  return out;
}

export function PhoneEra({ artistId, name }: { artistId: string; name: string }) {
  const { data: loaded, error, retry } = useLoad(() => backend.eraPage(artistId, name), [artistId, name]);
  const { data, reload } = useReload(loaded, () => backend.eraPage(artistId, name));
  useEffect(() => {
    const uns = [backend.listen("tags://changed", reload), backend.listen("library://changed", reload)];
    return () => uns.forEach((un) => un.then((f) => f()));
  }, [artistId, name, reload]);
  const nowId = usePlayer((s) => s.state?.track?.id);
  const ids = data ? data.groups.flatMap((g) => g.tracks.map((t) => t.id)) : [];
  const playingHere = usePlayer((s) => s.state?.queue.context?.kind === "tracks" && !!nowId && ids.includes(nowId) && s.state?.status === "playing");
  if (error && !data) return <PageFailed what="this era" retry={retry} framed />;
  if (!data) return <PageLoading />;
  const era = data.era;
  const range = eraRange(era);
  return (
    <PageFrame name="era" title={era.name} kicker="Era" className="ph-era">
      <Hero art={<Art path={era.cover} fluid requestSize={480} alt="" />}>
        <button type="button" className="ph-album__artist" onClick={() => nav.go({ name: "artist", id: artistId })}>
          {data.artist.title}
        </button>
        <p className="ph-x__meta tabular">{[range, `${plural(era.trackCount, "song")}, ${plural(era.albumCount, "release")}`].filter(Boolean).join(" · ")}</p>
      </Hero>
      <PlayRow
        name={name}
        playing={playingHere}
        onPlay={() => (playingHere ? void player.toggle() : void player.playTracks(ids))}
        onShuffle={() => void player.playTracks(shuffled(ids))}
        left={
          <button type="button" className="ph-link-btn" onClick={() => nav.go({ name: "artist", id: artistId, era: name })}>
            On the artist page
          </button>
        }
      />
      <EraStory data={data} className="ph-era__story" />
      {data.groups.map((g) => (
        <section key={g.album.id} className="ph-x__section" aria-label={g.album.title}>
          <button type="button" className="ph-row ph-era__group" onClick={() => nav.go({ name: "album", id: g.album.id })}>
            <Art path={g.album.thumb} size={56} sleeve={{ title: g.album.title, artist: g.album.artistTitle }} />
            <span className="ph-row__text">
              <span className="ph-row__title">{g.album.title}</span>
              <span className="ph-row__sub">
                {[releaseTypeLabel(g.album), g.date ? formatArchiveDate(g.date) : g.album.year ? String(g.album.year) : null, plural(g.tracks.length, "song"), g.edition].filter(Boolean).join(" · ")}
              </span>
            </span>
          </button>
          <ul className="ph-songs" aria-label={`${g.album.title} songs`}>
            {g.tracks.map((t, i) => (
              <SongRow key={t.id} track={t} index={i + 1} art={false} onPlay={() => void player.playTracks(ids, t.id)} />
            ))}
          </ul>
        </section>
      ))}
    </PageFrame>
  );
}
