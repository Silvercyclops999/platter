import { Suspense, lazy, useState } from "react";
import { CardSkeleton } from "../components/Card";
import { FilterChips } from "../components/Tabs";
import { Shelf, ShelfItem } from "../components/Shelf";
import { updateSettings, useSettings } from "../stores/settings";
import { activeChip, discographyChips, discographyItems, type DiscoEntry, type DiscographyFilter } from "./artist";
import { AlbumCard } from "./cards";
import { CatalogueAlbumCard } from "./catalogue";

/** Cards for releases Lidarr can get (and their request dialog), loaded once an artist has some. */
const MissingReleaseCard = lazy(() => import("./LidarrRequest").then((m) => ({ default: m.MissingReleaseCard })));

/**
 * The discography shelf: one row of cards, library releases and the ones you
 * can get alike, with chips that act like tabs (exactly one on). The chip
 * picked is remembered on this device for every artist; Show all opens the
 * full discography at it.
 */
export function DiscographyShelf({
  entries,
  lidarrReady,
  artistLibraryId,
  onChanged,
  onShowAll,
}: {
  entries: DiscoEntry[];
  lidarrReady: boolean;
  /** The library artist Get files releases under. */
  artistLibraryId?: string | null;
  /** A Get changed what a release you can get says. */
  onChanged?: () => void;
  onShowAll: (filter: DiscographyFilter) => void;
}) {
  const remembered = useSettings((s) => s.settings?.discographyChip);
  // The pick shows at once, before Settings has it.
  const [picked, setPicked] = useState<DiscographyFilter | null>(null);
  const chips = discographyChips(entries);
  const filter = activeChip(chips, picked ?? remembered);
  const pick = (id: DiscographyFilter) => {
    if (id === filter) return;
    setPicked(id);
    void updateSettings({ discographyChip: id });
  };
  const chipRow = <FilterChips label="Release type" items={chips} value={filter} onChange={pick} className="discography__chips" scroll />;
  return (
    <div className="discography">
      <Shelf title="Discography" onShowAll={() => onShowAll(filter)} filters={chipRow}>
        {discographyItems(entries, filter).map(({ entry, latest }) => (
          <ShelfItem key={entry.key}>
            {entry.album ? (
              <AlbumCard album={entry.album} subtitle="year" note={latest ? "Latest release" : undefined} />
            ) : entry.missing && artistLibraryId ? (
              <Suspense fallback={<CardSkeleton />}>
                <MissingReleaseCard item={entry.missing} artist={entry.cat!.artist} artistId={artistLibraryId} onChanged={onChanged ?? (() => {})} />
              </Suspense>
            ) : (
              <CatalogueAlbumCard
                album={entry.cat!}
                lidarrReady={lidarrReady}
                artistLibraryId={artistLibraryId}
                onChanged={onChanged}
                note={latest ? "Latest release" : undefined}
              />
            )}
          </ShelfItem>
        ))}
      </Shelf>
    </div>
  );
}
