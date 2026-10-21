/**
 * The small, always-loaded side of Lidarr requests: whether Request shows,
 * an artist's missing releases, and request state names. The cards and the
 * request dialog live in LidarrRequest.tsx, loaded when first needed.
 */
import { useCallback, useEffect, useState } from "react";
import { backend } from "../lib/backend";
import type { LidarrMissing, LidarrRequestState } from "../lib/types";
import { useCanRequest } from "../stores/asks";
import { useSettings } from "../stores/settings";

export const STATE_LABEL: Record<LidarrRequestState, string> = {
  queued: "Waiting to send",
  requested: "Requested",
  searching: "Looking on your indexers",
  downloading: "Downloading",
  importing: "Importing",
  imported: "In your library",
  failed: "Failed",
  notFound: "Not found yet",
};

/**
 * An artist's official releases the library lacks (from MusicBrainz). Empty
 * unless the setting is on, Lidarr is set up and its last check passed.
 */
export function useMissingReleases(artistId: string, artist: string | undefined) {
  const on = useSettings((s) => s.settings?.showMissingReleases !== false);
  const home = useSettings((s) => s.settings?.lidarrUrl);
  const away = useSettings((s) => s.settings?.lidarrAwayUrl);
  const [missing, setMissing] = useState<LidarrMissing | null>(null);
  const load = useCallback(() => {
    // The core says whether it's shown (Lidarr reachable directly or through the home side).
    if (!artist || !on) {
      setMissing(null);
      return () => {};
    }
    let alive = true;
    backend.lidarrMissingReleases(artistId, artist).then(
      (m) => alive && setMissing(m),
      () => alive && setMissing(null),
    );
    return () => {
      alive = false;
    };
  }, [artistId, artist, on, home, away]);
  useEffect(load, [load]);
  return { items: missing?.shown ? missing.items : [], reload: load };
}

/**
 * True when Request should show: Lidarr is reachable (its own addresses
 * with a key, or through a home-side computer or the Bridge) and its last
 * setup check passed. The core decides; every request still runs the check
 * again first. Asked again when an address changes or a Bridge comes or goes.
 */
export function useLidarrReady(): boolean {
  const home = useSettings((s) => s.settings?.lidarrUrl);
  const away = useSettings((s) => s.settings?.lidarrAwayUrl);
  const [ready, setReady] = useState(false);
  const [version, setVersion] = useState(0);
  useEffect(() => {
    const un = backend.listen("bridge://changed", () => setVersion((v) => v + 1));
    return () => void un.then((f) => f());
  }, []);
  useEffect(() => {
    let alive = true;
    backend.lidarrStatus().then(
      (s) => alive && setReady(s.ready),
      () => alive && setReady(false),
    );
    return () => {
      alive = false;
    };
  }, [home, away, version]);
  return ready;
}

/**
 * True when music you don't have shows Get (the owner, with Lidarr ready)
 * or Request (someone the server is shared with, when the owner lets them).
 * The buttons themselves pick which.
 */
export function useGetSpots(): boolean {
  const ready = useLidarrReady();
  const asking = useCanRequest();
  return ready || asking;
}
