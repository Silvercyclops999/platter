/**
 * Where an artist's discography comes from: the library's releases, and the
 * ones you can get. A library artist with albums of their own gets Lidarr's
 * missing releases; one with none (a featured guest, or a server artist with
 * only guest spots) gets the catalogue's releases, and a photo from it.
 */
import { useCallback, useEffect, useState } from "react";
import { backend } from "../lib/backend";
import type { FeaturedArtist } from "../lib/types";
import { catalogueEntries, libraryEntries, missingEntries, type DiscoEntry } from "./artist";
import { useMissingReleases } from "./lidarrShared";
import type { ArtistPageSection } from "../lib/types";

/** The catalogue's view of a featured guest (or any artist with no albums of their own); null until it answers or when `name` is unknown. */
export function useFeaturedArtist(id: string, name: string | undefined): { featured: FeaturedArtist | null; reload: () => void } {
  const [featured, setFeatured] = useState<FeaturedArtist | null>(null);
  const [version, setVersion] = useState(0);
  useEffect(() => {
    if (!name) {
      setFeatured(null);
      return;
    }
    let alive = true;
    backend.featuredArtist(id, name).then(
      (f) => alive && setFeatured(f),
      () => alive && setFeatured(null),
    );
    return () => {
      alive = false;
    };
  }, [id, name, version]);
  const reload = useCallback(() => setVersion((v) => v + 1), []);
  return { featured: featured?.shown ? featured : null, reload };
}

export interface ArtistDiscography {
  /** Library releases, then the ones you can get. */
  entries: DiscoEntry[];
  /** How many of them you can get. */
  catalogue: number;
  /** A photo from the catalogue, for an artist the server has none for. */
  photo: string | null;
  photoCredit: string | null;
  reload: () => void;
}

/** A library (or credited) artist's whole discography. `sections` is undefined while the page loads. */
export function useArtistDiscography(id: string, title: string | undefined, sections: ArtistPageSection[] | undefined): ArtistDiscography {
  const own = sections?.some((s) => s.albums.length);
  const missing = useMissingReleases(id, own ? title : undefined);
  const { featured, reload } = useFeaturedArtist(id, sections && !own ? title : undefined);
  const library = sections ? libraryEntries(sections) : [];
  const got =
    own && title
      ? missingEntries(missing.items, title)
      : featured
        ? catalogueEntries(featured.sections, [], new Set(library.map((e) => e.key)))
        : [];
  return {
    entries: [...library, ...got],
    catalogue: got.length,
    photo: featured?.photo ?? null,
    photoCredit: featured?.photoCredit ?? null,
    reload: own ? missing.reload : reload,
  };
}
