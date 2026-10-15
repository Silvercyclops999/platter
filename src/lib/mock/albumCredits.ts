/**
 * Made-up album credits for the mock backend, shaped like the core's
 * MusicBrainz answer: an original release date, a label, producers and
 * writers on the album, and per-song credits with the odd featured guest.
 * Every fifth album has no match and demos are never looked up, like the
 * core. Nothing here comes from a real service.
 */
import type { Album, AlbumCreditRow, AlbumCredits, Artist, Track } from "../types";

const PEOPLE = ["Nell Ardent", "Tobias Fenn", "Rosalind Kerr", "Mateo Ilves", "Wren Okonkwo", "Hal Brisco", "June Takara", "Ivo Sandeman"];
const LABELS = ["Northlight Records", "Low Tide Music", "Parlour Sound", "Greywater"];

export function mockAlbumCredits(album: Album, tracks: Track[], artists: Artist[], on: boolean): AlbumCredits {
  const empty = (state: AlbumCredits["state"]): AlbumCredits => ({
    albumId: album.id,
    state,
    released: null,
    editionDate: null,
    label: null,
    credits: [],
    tracks: [],
    url: null,
  });
  if (!on) return empty("off");
  const n = Number(album.id) || album.title.length;
  if (album.section === "demos" || n % 5 === 4) return empty("none");

  const person = (i: number) => PEOPLE[(n + i) % PEOPLE.length];
  // The album's own artist writes; one library artist produces some albums.
  const producerArtist = artists[(n + 2) % artists.length];
  const producer: AlbumCreditRow =
    n % 2 === 0 && producerArtist && producerArtist.id !== album.artistId
      ? { name: producerArtist.title, artistId: producerArtist.id, roles: ["producer"] }
      : { name: person(0), artistId: null, roles: ["producer", "mixer"] };
  const writers: AlbumCreditRow[] = [
    { name: album.artistTitle, artistId: album.artistId, roles: ["writer"] },
    { name: person(1), artistId: null, roles: ["writer", "composer"] },
  ];
  const crew: AlbumCreditRow[] = [
    { name: person(3), artistId: null, roles: ["engineer"] },
    { name: person(4), artistId: null, roles: ["mastering"] },
  ];
  const guest = artists[(n + 5) % artists.length];
  const perTrack = tracks.map((t, i) => {
    const credits: AlbumCreditRow[] = [
      { name: producer.name, artistId: producer.artistId, roles: ["producer"] },
      { name: album.artistTitle, artistId: album.artistId, roles: ["writer"] },
    ];
    if (i % 3 === 1) credits.push({ name: person(1), artistId: null, roles: ["writer", "composer"] });
    if (i % 4 === 2 && guest && guest.id !== album.artistId) credits.push({ name: guest.title, artistId: guest.id, roles: ["featured"] });
    return { trackId: t.id, credits };
  });
  const year = album.year ?? 2000 + (n % 20);
  const month = String((n % 12) + 1).padStart(2, "0");
  const day = String((n % 27) + 1).padStart(2, "0");
  return {
    albumId: album.id,
    state: "found",
    released: `${year}-${month}-${day}`,
    editionDate: n % 3 === 0 ? `${year + 10}-${month}-01` : null,
    label: LABELS[n % LABELS.length],
    credits: [producer, ...writers, ...crew],
    tracks: perTrack,
    url: `https://musicbrainz.org/release/00000000-0000-4000-8000-${String(n).padStart(12, "0")}`,
  };
}
