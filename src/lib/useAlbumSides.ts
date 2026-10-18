import { useEffect, useState } from "react";
import { useSettings } from "../stores/settings";
import { backend } from "./backend";
import type { AlbumSide, AlbumSides } from "./types";

/** The side a song is on, or null. */
export function sideOf(sides: AlbumSide[] | null | undefined, trackId: string | null | undefined): AlbumSide | null {
  if (!trackId) return null;
  return sides?.find((s) => s.trackIds.includes(trackId)) ?? null;
}

/**
 * An album's record sides while "Show vinyl sides" is on (null before the
 * answer, when it's off, or with no album). Asks again when the sleeve
 * notes' lookups finish (a vinyl edition may have turned up) and when a
 * split by hand changes, here or on another device.
 */
export function useAlbumSides(albumId: string | null | undefined): [AlbumSides | null, (s: AlbumSides) => void] {
  const on = useSettings((s) => s.settings?.showVinylSides !== false);
  const [data, setData] = useState<AlbumSides | null>(null);
  useEffect(() => {
    setData(null);
    if (!albumId || !on) return;
    let alive = true;
    const load = () =>
      backend.albumSides(albumId).then(
        (d) => alive && setData(d),
        () => {},
      );
    void load();
    const offSleeve = backend.listen("sleeve://changed", (e) => {
      if (e.id === albumId) void load();
    });
    const offStore = backend.listen("syncstore://changed", (c) => {
      if (c.namespaces.includes("albumSides")) void load();
    });
    return () => {
      alive = false;
      void offSleeve.then((f) => f());
      void offStore.then((f) => f());
    };
  }, [albumId, on]);
  return [on && data?.albumId === albumId ? data : null, setData];
}
