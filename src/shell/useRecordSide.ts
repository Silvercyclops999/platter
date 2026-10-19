/**
 * The side of the record the playing song is on: the songs' lengths in
 * order and which one is playing. Uses the album's own sides when it has
 * them; otherwise the album's songs (on the song's disc) split into sides of
 * a real length. Albums already read are kept for the session.
 */
import { useEffect, useState } from "react";
import { backend } from "../lib/backend";
import { useAlbumSides } from "../lib/useAlbumSides";
import type { Track } from "../lib/types";
import type { RecordSide } from "../components/Record";
import { splitSides } from "./recordGeometry";

type Song = Pick<Track, "id" | "durationMs" | "discNumber">;

const albums = new Map<string, Song[]>();

/** The side `trackId` is on, from the album's songs and its sides (if known). */
export function sideFor(songs: readonly Song[], trackId: string, sides?: { trackIds: string[] }[] | null): RecordSide | null {
  const known = sides?.find((s) => s.trackIds.includes(trackId));
  if (known) {
    const byId = new Map(songs.map((s) => [s.id, s]));
    const list = known.trackIds.map((id) => byId.get(id)).filter((s): s is Song => !!s);
    const current = list.findIndex((s) => s.id === trackId);
    if (current >= 0) return { durationsMs: list.map((s) => s.durationMs), current };
  }
  const me = songs.find((s) => s.id === trackId);
  if (!me) return null;
  const disc = songs.filter((s) => s.discNumber === me.discNumber);
  const at = disc.indexOf(me);
  for (const idx of splitSides(disc.map((s) => s.durationMs))) {
    if (idx.includes(at)) return { durationsMs: idx.map((i) => disc[i].durationMs), current: idx.indexOf(at) };
  }
  return null;
}

export function useRecordSide(track: Pick<Track, "id" | "albumId" | "durationMs"> | null | undefined): RecordSide | null {
  const albumId = track?.albumId ?? null;
  const [songs, setSongs] = useState<{ albumId: string; songs: Song[] } | null>(() =>
    albumId && albums.has(albumId) ? { albumId, songs: albums.get(albumId)! } : null,
  );
  const [sides] = useAlbumSides(albumId);
  useEffect(() => {
    if (!albumId) return;
    const have = albums.get(albumId);
    if (have) {
      setSongs({ albumId, songs: have });
      return;
    }
    let alive = true;
    backend.albumPage(albumId).then(
      (p) => {
        const list = p.tracks.map((t) => ({ id: t.id, durationMs: t.durationMs, discNumber: t.discNumber }));
        albums.set(albumId, list);
        if (alive) setSongs({ albumId, songs: list });
      },
      () => {},
    );
    return () => {
      alive = false;
    };
  }, [albumId]);
  if (!track) return null;
  const list = songs?.albumId === albumId ? songs.songs : null;
  const side = list ? sideFor(list, track.id, sides?.sides) : null;
  // Until the album is read (or if it can't be), the song alone fills the side.
  return side ?? (track.durationMs > 0 ? { durationsMs: [track.durationMs], current: 0 } : null);
}
