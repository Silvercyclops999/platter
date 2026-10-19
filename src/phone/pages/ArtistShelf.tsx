/**
 * An artist page shelf's Show all on the phone: Appears on (records, with
 * their artist) or Fans also like (library artists first, then artists that
 * aren't in the library, which open their catalogue page).
 */
import { backend } from "../../lib/backend";
import { useLoad } from "../../lib/useLoad";
import { useCatalogueFans } from "../../pages/catalogue";
import { ARTIST_SHELF_TITLES, type ArtistShelfId } from "../../pages/HomeShelfPage";
import { nav } from "../../stores/router";
import { PageFailed } from "./common";
import { CataloguePhoto, Empty, GhostRows, PageFrame, Tile, TileGrid } from "./kit";

export function PhoneArtistShelf({ id, shelf }: { id: string; shelf: ArtistShelfId }) {
  const { data, error, retry } = useLoad(() => backend.artistPage(id), [id]);
  const more = useCatalogueFans(shelf === "fansAlsoLike" ? id : null, data?.artist.title);
  const title = ARTIST_SHELF_TITLES[shelf];
  const count = data ? (shelf === "appearsOn" ? (data.appearsOn ?? []).length : data.similar.length + more.length) : 0;
  return (
    <PageFrame name="artistShelf" title={title} kicker={data?.artist.title}>
      {error && !data ? (
        <PageFailed what={title} retry={retry} />
      ) : !data ? (
        <GhostRows />
      ) : count === 0 ? (
        <Empty title="Nothing here yet" />
      ) : shelf === "appearsOn" ? (
        <TileGrid label={title}>
          {(data.appearsOn ?? []).map((a) => (
            <Tile key={a.id} title={a.title} sub={a.artistTitle} thumb={a.thumb} onOpen={() => nav.go({ name: "album", id: a.id })} />
          ))}
        </TileGrid>
      ) : (
        <TileGrid label={title} round>
          {data.similar.map((a) => (
            <Tile key={a.id} title={a.title} sub="Artist" thumb={a.thumb} round kind="artist" onOpen={() => nav.go({ name: "artist", id: a.id })} />
          ))}
          {more.map((a) => (
            <Tile
              key={`mb-${a.mbid}`}
              title={a.name}
              sub="Not in your library"
              art={<CataloguePhoto mbid={a.libraryId ? null : a.mbid} name={a.name} given={a.photo} fluid />}
              round
              kind="artist"
              onOpen={() => (a.libraryId ? nav.go({ name: "artist", id: a.libraryId }) : nav.go({ name: "catalogueArtist", mbid: a.mbid }))}
            />
          ))}
        </TileGrid>
      )}
    </PageFrame>
  );
}
