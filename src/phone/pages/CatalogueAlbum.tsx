/**
 * A record that isn't in the library, on the phone: its cover and words,
 * Get (Lidarr) and Add to Missing, where to buy it, and the tracklist of its
 * first official release (nothing to play yet).
 */
import { Check, ExternalLink, ListPlus } from "lucide-react";
import { Fragment, useEffect, useState } from "react";
import { Art } from "../../components/Art";
import { backend } from "../../lib/backend";
import { catalogueKind, formatDuration, formatLongDuration, plural } from "../../lib/format";
import { STORE_LINKS } from "../../lib/importStrings";
import { useLoad } from "../../lib/useLoad";
import { CatalogueAction, addToMissing, openStore, removeFromMissing } from "../../pages/catalogue";
import { PHONE_SET_UP, useGetSpots, useLidarrNote } from "../../pages/lidarrShared";
import { useIsGuest } from "../../stores/asks";
import { nav, useRouter } from "../../stores/router";
import { PageFailed, PageLoading } from "./common";
import { Hero, PageFrame } from "./kit";

export function PhoneCatalogueAlbum({ id }: { id: string }) {
  const [version, setVersion] = useState(0);
  const { data, error, retry } = useLoad(() => backend.catalogueAlbum(id), [id, version]);
  const lidarrReady = useGetSpots();
  const lidarrSays = useLidarrNote(PHONE_SET_UP);
  const guest = useIsGuest();
  const [busy, setBusy] = useState(false);
  const owned = data?.album.libraryAlbumId;
  useEffect(() => {
    if (owned) useRouter.getState().replace({ name: "album", id: owned });
  }, [owned]);
  if (error && !data) return <PageFailed what="this album" retry={retry} framed />;
  if (!data) return <PageLoading />;
  const album = data.album;
  const reload = () => setVersion((v) => v + 1);
  const total = data.tracks.reduce((n, t) => n + (t.durationMs ?? 0), 0);
  const discs = new Set(data.tracks.map((t) => t.disc ?? 1)).size > 1;
  const goArtist = () => {
    if (data.artistLibraryId) nav.go({ name: "artist", id: data.artistLibraryId });
    else if (album.artistMbid) nav.go({ name: "catalogueArtist", mbid: album.artistMbid });
  };
  const missing = async () => {
    setBusy(true);
    const ok = album.inMissing ? await removeFromMissing(album) : await addToMissing(album);
    setBusy(false);
    if (ok) reload();
  };
  return (
    <PageFrame name="catalogueAlbum" title={album.title} kicker={`${catalogueKind(album)} · Not in your library`} className="ph-cat">
      <Hero art={<Art path={album.thumb} fluid requestSize={480} alt="" sleeve={{ title: album.title, artist: album.artist }} />}>
        <button type="button" className="ph-album__artist" onClick={goArtist}>
          {album.artist}
        </button>
        <p className="ph-x__meta tabular">{[album.year, data.tracks.length ? plural(data.tracks.length, "song") : null, total ? formatLongDuration(total) : null].filter(Boolean).join(" · ")}</p>
      </Hero>
      <div className="ph-x__pill-row ph-get ph-cat__actions">
        <CatalogueAction album={album} lidarrReady={lidarrReady} artistLibraryId={data.artistLibraryId} onChanged={reload} size="md" />
        <button type="button" className={`ph-pill ${album.inMissing ? "is-on" : ""}`} aria-pressed={album.inMissing} disabled={busy} onClick={() => void missing()}>
          {album.inMissing ? <Check size={16} aria-hidden="true" /> : <ListPlus size={16} aria-hidden="true" />}
          {album.inMissing ? "On your Missing list" : "Add to Missing"}
        </button>
      </div>
      {album.refusal ? <p className="ph-x__note">{album.refusal}</p> : !lidarrReady && lidarrSays && !guest && <p className="ph-x__note">{lidarrSays}</p>}
      <section className="ph-x__section" aria-label={`Buy ${album.title}`}>
        <h2 className="ph-label">Buy or find it</h2>
        <div className="ph-x__pill-row">
          {STORE_LINKS.map((st) => (
            <button key={st.key} type="button" className="ph-pill" aria-label={`Search ${st.name} for ${album.title}`} onClick={() => void openStore(data.links[st.key])}>
              <ExternalLink size={14} aria-hidden="true" /> {st.name}
            </button>
          ))}
        </div>
      </section>
      {data.tracks.length > 0 ? (
        <ol className="ph-songs ph-cat__tracks" aria-label={`${album.title} songs, not in your library`}>
          {data.tracks.map((t, i) => (
            <Fragment key={`${t.disc ?? 1}-${t.number}-${i}`}>
              {discs && (i === 0 || data.tracks[i - 1].disc !== t.disc) && (
                <li className="ph-disc-head" aria-hidden="true">
                  Disc {t.disc ?? 1}
                </li>
              )}
              <li className="ph-song ph-cat__track">
                <span className="ph-song__num tabular">{t.number}</span>
                <span className="ph-row__text">
                  <span className="ph-row__title">{t.title}</span>
                  {t.artist && t.artist !== album.artist && <span className="ph-row__sub">{t.artist}</span>}
                </span>
                {t.durationMs != null && <span className="ph-x__count">{formatDuration(t.durationMs)}</span>}
              </li>
            </Fragment>
          ))}
        </ol>
      ) : (
        <p className="ph-x__note">MusicBrainz has no tracklist for this album yet.</p>
      )}
    </PageFrame>
  );
}
