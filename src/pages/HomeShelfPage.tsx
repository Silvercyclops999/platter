import type { ReactNode } from "react";
import { CardSkeleton } from "../components/Card";
import { backend } from "../lib/backend";
import type { HomeShelfId } from "../lib/types";
import { useLoad } from "../lib/useLoad";
import { AlbumCard, ArtistCard, HomeItemCard, homeItemInfo } from "./cards";
import { shelfTitle } from "./home";
import "./home.css";

/**
 * A shelf's "Show all": a plain page with the shelf's title and the same
 * cards wrapped into rows. No header art, play button or sort and filter
 * controls; the list is the shelf's full, fixed list.
 */
export function ShowAllGrid({
  title,
  error,
  empty,
  skeletonKind = "album",
  children,
}: {
  title: string;
  error?: { message: string } | null;
  /** Loaded with nothing in it. */
  empty?: boolean;
  skeletonKind?: "album" | "artist";
  /** The cards, or undefined while loading. */
  children: ReactNode[] | undefined;
}) {
  return (
    <div className="page-pad shelf-page">
      <h1 className="page-title">{title}</h1>
      {error && !children ? (
        <p className="page-error">
          Couldn't load {title}: {error.message}
        </p>
      ) : empty ? (
        <p className="home__empty" role="status">
          Nothing here yet.
        </p>
      ) : (
        <div className="shelf-page__grid" role="list" aria-label={title} aria-busy={!children || undefined}>
          {children ??
            Array.from({ length: 12 }, (_, i) => (
              <div key={i} aria-hidden="true">
                <CardSkeleton kind={skeletonKind} />
              </div>
            ))}
        </div>
      )}
    </div>
  );
}

/** A Home shelf's "Show all". Recently played stays a grid for now. */
export function HomeShelfPage({ shelf }: { shelf: HomeShelfId }) {
  const { data, error } = useLoad(() => backend.homeShelf(shelf), [shelf]);
  return (
    <ShowAllGrid title={shelfTitle(shelf)} error={error} empty={data?.length === 0} skeletonKind={shelf === "topArtists" ? "artist" : "album"}>
      {data?.map((item) => (
        <div role="listitem" key={homeItemInfo(item).key}>
          <HomeItemCard item={item} />
        </div>
      ))}
    </ShowAllGrid>
  );
}

export type ArtistShelfId = "appearsOn" | "fansAlsoLike";

export const ARTIST_SHELF_TITLES: Record<ArtistShelfId, string> = {
  appearsOn: "Appears on",
  fansAlsoLike: "Fans also like",
};

/** An artist page shelf's "Show all": albums keep their artist line, artists their round art. */
export function ArtistShelfPage({ id, shelf }: { id: string; shelf: ArtistShelfId }) {
  const { data, error } = useLoad(() => backend.artistPage(id), [id]);
  const title = ARTIST_SHELF_TITLES[shelf];
  const cards =
    data &&
    (shelf === "appearsOn"
      ? (data.appearsOn ?? []).map((album) => (
          <div role="listitem" key={album.id}>
            <AlbumCard album={album} subtitle="artist" />
          </div>
        ))
      : data.similar.map((artist) => (
          <div role="listitem" key={artist.id}>
            <ArtistCard artist={artist} />
          </div>
        )));
  return (
    <ShowAllGrid title={title} error={error} empty={cards?.length === 0} skeletonKind={shelf === "fansAlsoLike" ? "artist" : "album"}>
      {cards}
    </ShowAllGrid>
  );
}
