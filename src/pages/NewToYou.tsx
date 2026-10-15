/**
 * "New to you": artists like the ones you play most that aren't in your
 * library (ListenBrainz similar artists and MusicBrainz relationships), as
 * a Home shelf and a page. "Not interested" hides an artist for good (undo
 * from the toast or in Settings).
 */
import { useEffect, useState } from "react";
import { Shelf, ShelfItem } from "../components/Shelf";
import { CardSkeleton } from "../components/Card";
import { backend } from "../lib/backend";
import type { CatArtist, NewArtist, NewToYou } from "../lib/types";
import { nav } from "../stores/router";
import { useSettings } from "../stores/settings";
import { CatalogueArtistCard, useDismissed, useOnline } from "./catalogue";
import "./catalogue.css";

/** Most cards on the Home shelf. */
const SHELF_MAX = 12;

const asArtist = (a: NewArtist): CatArtist => ({ mbid: a.mbid, name: a.name, disambiguation: null, kind: null, photo: null, libraryId: null });

/** The list, with "Not interested" applied at once. Nothing while the setting is off or offline. */
export function useNewToYou(): NewToYou | null {
  const on = useSettings((s) => s.settings?.homeNewToYou !== false);
  const online = useOnline();
  const dismissed = useDismissed();
  const [data, setData] = useState<NewToYou | null>(null);
  useEffect(() => {
    if (!on || !online) {
      setData(null);
      return;
    }
    let alive = true;
    backend.newToYou().then(
      (d) => alive && setData(d),
      () => alive && setData(null),
    );
    return () => {
      alive = false;
    };
  }, [on, online]);
  if (!data) return null;
  const gone = new Set(dismissed.map((d) => d.mbid));
  return { ...data, artists: data.artists.filter((a) => !gone.has(a.mbid)) };
}

/** Home's shelf. Hidden while off, offline or empty. */
export function NewToYouShelf() {
  const data = useNewToYou();
  if (!data?.shown || data.artists.length === 0) return null;
  return (
    <Shelf title="New to you" skeletonKind="artist" onShowAll={() => nav.go({ name: "newToYou" })}>
      {data.artists.slice(0, SHELF_MAX).map((a) => (
        <ShelfItem key={a.mbid}>
          <CatalogueArtistCard artist={asArtist(a)} because={a.because[0] ?? null} onDismiss={() => {}} />
        </ShelfItem>
      ))}
    </Shelf>
  );
}

export function NewToYouPage() {
  const data = useNewToYou();
  const on = useSettings((s) => s.settings?.homeNewToYou !== false);
  return (
    <div className="page-pad">
      <h1 className="page-title">New to you</h1>
      <p className="import-page__sub">Artists like the ones you play most that aren't in your library. Open one to see their albums, request them or find them in a store.</p>
      {!on ? (
        <p className="ntu-empty" role="status">
          New to you is off. Turn it on in Settings.
        </p>
      ) : !data ? (
        <div className="ntu-grid" aria-hidden="true">
          {Array.from({ length: 8 }, (_, i) => (
            <CardSkeleton key={i} kind="artist" />
          ))}
        </div>
      ) : data.artists.length === 0 ? (
        <p className="ntu-empty" role="status">
          Nothing new yet. Play some more and check back.
        </p>
      ) : (
        <div className="ntu-grid" role="list" aria-label="New to you">
          {data.artists.map((a) => (
            <div role="listitem" key={a.mbid}>
              <CatalogueArtistCard artist={asArtist(a)} because={a.because[0] ?? null} onDismiss={() => {}} />
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
