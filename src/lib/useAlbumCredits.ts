import { useEffect, useState } from "react";
import { useSettings } from "../stores/settings";
import { backend } from "./backend";
import type { AlbumCredits, AlbumCreditRow } from "./types";

/**
 * An album's MusicBrainz date and credits. Answers from the core's cache at
 * once; reloads when the core says a lookup finished or the setting changes.
 * Failures leave the page on Plex data only (null).
 */
export function useAlbumCredits(albumId: string | null | undefined): AlbumCredits | null {
  const [credits, setCredits] = useState<AlbumCredits | null>(null);
  const on = useSettings((s) => s.settings?.infoMusicbrainz ?? false);
  useEffect(() => {
    setCredits(null);
    if (!albumId || !on) return;
    let alive = true;
    const load = () =>
      backend.albumCredits(albumId).then(
        (c) => alive && setCredits(c),
        () => {},
      );
    void load();
    const un = backend.listen("albumcredits://changed", (e) => {
      if (e.id === albumId) void load();
    });
    return () => {
      alive = false;
      un.then((f) => f());
    };
  }, [albumId, on]);
  return credits;
}

/** One song's MusicBrainz credits, if the album's lookup matched it. */
export function songCredits(credits: AlbumCredits | null | undefined, trackId: string): AlbumCreditRow[] {
  return credits?.tracks.find((t) => t.trackId === trackId)?.credits ?? [];
}
