import { CardSkeleton } from "../components/Card";
import { backend } from "../lib/backend";
import type { HomeShelfId } from "../lib/types";
import { useLoad } from "../lib/useLoad";
import { HomeItemCard, homeItemInfo } from "./cards";
import { shelfTitle } from "./home";
import "./home.css";

/** A Home shelf's "Show all": everything in it as a card grid. */
export function HomeShelfPage({ shelf }: { shelf: HomeShelfId }) {
  const { data, error } = useLoad(() => backend.homeShelf(shelf), [shelf]);
  const title = shelfTitle(shelf);
  return (
    <div className="page-pad shelf-page">
      <h1 className="page-title">{title}</h1>
      {error && !data ? (
        <p className="page-error">
          Couldn't load {title}: {error.message}
        </p>
      ) : data && data.length === 0 ? (
        <p className="home__empty" role="status">
          Nothing here yet.
        </p>
      ) : (
        <div className="shelf-page__grid" role="list" aria-label={title} aria-busy={!data || undefined}>
          {data
            ? data.map((item) => (
                <div role="listitem" key={homeItemInfo(item).key}>
                  <HomeItemCard item={item} />
                </div>
              ))
            : Array.from({ length: 12 }, (_, i) => (
                <div key={i} aria-hidden="true">
                  <CardSkeleton kind={shelf === "topArtists" ? "artist" : "album"} />
                </div>
              ))}
        </div>
      )}
    </div>
  );
}
