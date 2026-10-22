/**
 * An artist that isn't in the library, on the phone: their photo and name,
 * Get albums (Lidarr) and Not interested, the discography with Get on each
 * record, About, and Fans also like. An artist the library turns out to
 * have opens their own page instead.
 */
import { Download } from "lucide-react";
import { useEffect, useState } from "react";
import { backend } from "../../lib/backend";
import { useLoad } from "../../lib/useLoad";
import { catalogueEntries } from "../../pages/artist";
import { dismissArtist, undismissArtist } from "../../pages/catalogue";
import { GetAlbumsDialog } from "../../pages/GetAlbumsDialog";
import { lidarrNote, useGetSpots, useLidarrStatus } from "../../pages/lidarrShared";
import { useIsGuest } from "../../stores/asks";
import { nav, useRouter } from "../../stores/router";
import { useSettings } from "../../stores/settings";
import { PageFailed, PageLoading } from "./common";
import { DiscoShelf } from "./DiscoShelf";
import { CataloguePhoto, Hero, PageFrame, Tile, TileGrid } from "./kit";

export function PhoneCatalogueArtist({ mbid }: { mbid: string }) {
  const [version, setVersion] = useState(0);
  const { data, error, retry } = useLoad(() => backend.catalogueArtist(mbid), [mbid, version]);
  const lidarr = useLidarrStatus();
  const lidarrReady = !!lidarr?.ready;
  const lidarrSays = lidarrNote(lidarr, "Set up Lidarr in Settings to get albums, or buy them from a store.");
  const spots = useGetSpots();
  const guest = useIsGuest();
  const [getting, setGetting] = useState(false);
  const dismissed = useSettings((s) => s.settings?.dismissedArtists?.some((d) => d.mbid === mbid)) ?? false;
  const libraryId = data?.artist.libraryId;
  useEffect(() => {
    if (libraryId) useRouter.getState().replace({ name: "artist", id: libraryId });
  }, [libraryId]);
  if (error && !data) return <PageFailed what="this artist" retry={retry} framed />;
  if (!data) return <PageLoading />;
  const artist = data.artist;
  const meta = [artist.disambiguation, data.area, data.lifeSpan].filter(Boolean).join(" · ");
  const reload = () => setVersion((v) => v + 1);
  return (
    <PageFrame name="catalogueArtist" title={artist.name} kicker="Not in your library" className="ph-cat">
      <Hero art={<CataloguePhoto mbid={artist.mbid} name={artist.name} given={artist.photo} fluid />}>
        {meta && <p className="ph-x__meta">{meta}</p>}
      </Hero>
      <div className="ph-x__pill-row ph-get">
        {lidarrReady && (
          <button type="button" className="ph-pill is-primary" onClick={() => setGetting(true)}>
            <Download size={16} aria-hidden="true" /> Get albums
          </button>
        )}
        <button
          type="button"
          className={`ph-pill ${dismissed ? "is-on" : ""}`}
          aria-pressed={dismissed}
          onClick={() => (dismissed ? undismissArtist(artist.mbid) : dismissArtist({ mbid: artist.mbid, name: artist.name }))}
        >
          {dismissed ? "Hidden from suggestions" : "Not interested"}
        </button>
      </div>
      {!lidarrReady && lidarrSays && !guest && <p className="ph-x__note">{lidarrSays}</p>}
      {getting && <GetAlbumsDialog mbid={artist.mbid} name={artist.name} open onClose={() => setGetting(false)} />}
      {data.sections.length === 0 ? (
        <p className="ph-x__note">MusicBrainz has no official albums for this artist yet.</p>
      ) : (
        <DiscoShelf
          entries={catalogueEntries(data.sections, data.popular)}
          lidarrReady={spots}
          onChanged={reload}
          onShowAll={(f) => nav.go({ name: "discography", id: artist.mbid, catalogue: true, ...(f === "popular" ? {} : { type: f }) })}
        />
      )}
      <section className="ph-x__section" aria-label="About">
        <h2 className="ph-section-title">About</h2>
        <p className="ph-cat__bio">{data.bio ?? `There's no biography for ${artist.name} yet.`}</p>
        {data.bioUrl && (
          <button type="button" className="ph-link-btn" onClick={() => void backend.openInfoLink(data.bioUrl!).catch(() => {})}>
            From {data.bioCredit ?? "Wikipedia"}
          </button>
        )}
      </section>
      {data.similar.length > 0 && (
        <section className="ph-x__section" aria-label="Fans also like">
          <h2 className="ph-section-title">Fans also like</h2>
          <TileGrid label="Fans also like" round>
            {data.similar.map((a) => (
              <Tile
                key={a.mbid}
                title={a.name}
                sub={a.libraryId ? "In your library" : undefined}
                art={<CataloguePhoto mbid={a.libraryId ? null : a.mbid} name={a.name} given={a.photo} fluid />}
                round
                kind="artist"
                onOpen={() => (a.libraryId ? nav.go({ name: "artist", id: a.libraryId }) : nav.go({ name: "catalogueArtist", mbid: a.mbid }))}
              />
            ))}
          </TileGrid>
        </section>
      )}
    </PageFrame>
  );
}
