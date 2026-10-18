/**
 * Under an album's tracklist: "More from <label>", "More produced by
 * <producer>", "Also recorded at <studio>", from the library only. Two
 * shelves at most; any other crates the album is in are links under
 * "More crates". Nothing shows when the album is alone in every crate.
 */
import { useEffect, useState } from "react";
import { Shelf, ShelfItem } from "../components/Shelf";
import { backend } from "../lib/backend";
import type { AlbumShelf } from "../lib/types";
import { useApp } from "../stores/app";
import { nav } from "../stores/router";
import { useSettings } from "../stores/settings";
import { AlbumCard } from "./cards";
import { SHELVES_SHOWN, records, shelfTitle } from "./crates";
import { RecordShelf, SleeveCard, useStudioLook } from "./RecordHome";
import "./record-home.css";
import "./crates.css";

const lineFor = (s: AlbumShelf) => `${records(s.count)} in this crate, this one included.`;

export function CrateShelves({ albumId }: { albumId: string }) {
  const on = useSettings((s) => s.settings?.showCrates !== false && s.settings?.crateShelves !== false);
  const studio = useStudioLook();
  const libraryVersion = useApp((s) => s.libraryVersion);
  const [shelves, setShelves] = useState<AlbumShelf[] | null>(null);
  useEffect(() => {
    setShelves(null);
    if (!on) return;
    let alive = true;
    backend.albumCrates(albumId).then(
      (s) => alive && setShelves(s),
      () => alive && setShelves([]),
    );
    return () => {
      alive = false;
    };
  }, [albumId, on, libraryVersion]);
  if (!on || !shelves?.length) return null;
  const shown = shelves.slice(0, SHELVES_SHOWN);
  const rest = shelves.slice(SHELVES_SHOWN);
  const open = (s: AlbumShelf) => nav.go({ name: "crate", kind: s.kind, key: s.key });
  return (
    <div className="crate-shelves detail-more" data-testid="crate-shelves">
      {shown.map((s) =>
        studio ? (
          <RecordShelf
            key={`${s.kind}:${s.key}`}
            title={shelfTitle(s)}
            line={lineFor(s)}
            testId={`crate-shelf-${s.kind}`}
            open={{ label: "Open crate", run: () => open(s) }}
          >
            {s.albums.map((a) => (
              <SleeveCard key={a.id} album={a} meta={a.year ? String(a.year) : "No year"} />
            ))}
          </RecordShelf>
        ) : (
          <Shelf key={`${s.kind}:${s.key}`} title={shelfTitle(s)} showAllLabel="Open crate" onShowAll={() => open(s)}>
            {s.albums.map((a) => (
              <ShelfItem key={a.id}>
                <AlbumCard album={a} />
              </ShelfItem>
            ))}
          </Shelf>
        ),
      )}
      {rest.length > 0 && (
        <nav className="crate-more" aria-label="More crates">
          <span className="crate-more__label">More crates</span>
          {rest.map((s) => (
            <button key={`${s.kind}:${s.key}`} type="button" className="crate-more__link" onClick={() => open(s)}>
              {shelfTitle(s)} <span className="crate-more__count">({s.count})</span>
            </button>
          ))}
        </nav>
      )}
    </div>
  );
}
