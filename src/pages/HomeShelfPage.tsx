import type { ReactNode } from "react";
import { CardSkeleton } from "../components/Card";
import { backend } from "../lib/backend";
import type { HomeShelfId } from "../lib/types";
import { useLoad } from "../lib/useLoad";
import { AlbumCard, ArtistCard, HomeItemCard, homeItemInfo } from "./cards";
import { playableOffline, shelfTitle } from "./home";
import { useCachedLibrary } from "./cachedLibrary";
import { useApp } from "../stores/app";
import { CatalogueArtistCard, useCatalogueFans } from "./catalogue";
import "./home.css";
import { LoadError } from "../components/LoadError";

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
  onRetry,
}: {
  title: string;
  error?: { message: string } | null;
  onRetry?: () => void;
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
        <LoadError what={title} error={error} onRetry={onRetry} inline />
      ) : empty ? (
        <div className="empty-state" role="status">
          <h2 className="empty-state__title">Nothing here yet</h2>
          <p className="empty-state__text">Play music from your library and this fills in.</p>
        </div>
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

/** A Home shelf's "Show all". Recently played stays a grid for now. Offline, only what can play. */
export function HomeShelfPage({ shelf }: { shelf: HomeShelfId }) {
  const { data: full, error, retry } = useLoad(() => backend.homeShelf(shelf), [shelf]);
  const cachedOnly = useApp((s) => !!s.health?.cachedOnly);
  const { lib, failed } = useCachedLibrary();
  // What's on this computer couldn't be read: the whole list, as connected.
  const data = full && cachedOnly && !failed ? (lib ? full.filter((i) => playableOffline(i, lib)) : undefined) : full;
  return (
    <ShowAllGrid title={shelfTitle(shelf)} error={error} onRetry={retry} empty={data?.length === 0} skeletonKind={shelf === "topArtists" ? "artist" : "album"}>
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
  const { data, error, retry } = useLoad(() => backend.artistPage(id), [id]);
  const more = useCatalogueFans(shelf === "fansAlsoLike" ? id : null, data?.artist.title);
  const title = ARTIST_SHELF_TITLES[shelf];
  const cards =
    data &&
    (shelf === "appearsOn"
      ? (data.appearsOn ?? []).map((album) => (
          <div role="listitem" key={album.id}>
            <AlbumCard album={album} subtitle="artist" />
          </div>
        ))
      : [
          ...data.similar.map((artist) => (
            <div role="listitem" key={artist.id}>
              <ArtistCard artist={artist} />
            </div>
          )),
          ...more.map((a) => (
            <div role="listitem" key={`mb-${a.mbid}`}>
              <CatalogueArtistCard artist={a} />
            </div>
          )),
        ]);
  return (
    <ShowAllGrid title={title} error={error} onRetry={retry} empty={cards?.length === 0} skeletonKind={shelf === "fansAlsoLike" ? "artist" : "album"}>
      {cards}
    </ShowAllGrid>
  );
}
