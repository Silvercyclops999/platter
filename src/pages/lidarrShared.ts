/**
 * The small, always-loaded side of Lidarr requests: whether Request shows,
 * an artist's missing releases, and request state names. The cards and the
 * request dialog live in LidarrRequest.tsx, loaded when first needed.
 */
import { useCallback, useEffect, useState } from "react";
import { backend } from "../lib/backend";
import type { LidarrMissing, LidarrRequestState, LidarrStatus } from "../lib/types";
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
 * Lidarr's state here, asked again when an address changes, a Bridge comes
 * or goes, or a setup check that started by itself finishes. Null until the
 * first answer.
 */
export function useLidarrStatus(): LidarrStatus | null {
  const home = useSettings((s) => s.settings?.lidarrUrl);
  const away = useSettings((s) => s.settings?.lidarrAwayUrl);
  const [status, setStatus] = useState<LidarrStatus | null>(null);
  const [version, setVersion] = useState(0);
  useEffect(() => {
    // A Bridge paired, Plex answering at last (who owns the server is known
    // then), or the setup check the core ran by itself.
    const again = () => setVersion((v) => v + 1);
    const offs = [backend.listen("bridge://changed", again), backend.listen("auth://changed", again), backend.listen("lidarr://changed", again)];
    return () => offs.forEach((un) => void un.then((f) => f()));
  }, []);
  useEffect(() => {
    let alive = true;
    backend.lidarrStatus().then(
      (s) => alive && setStatus(s),
      () => alive && setStatus(null),
    );
    return () => {
      alive = false;
    };
  }, [home, away, version]);
  return status;
}

/**
 * True when Request should show: Lidarr is reachable (its own addresses
 * with a key, or through a home-side computer or the Bridge) and its last
 * setup check passed. The core decides; every request still runs the check
 * again first.
 */
export function useLidarrReady(): boolean {
  return useLidarrStatus()?.ready ?? false;
}

/** Whether Lidarr can be asked from here at all (set up here, or a Bridge or a computer at home answers). */
export function lidarrReachable(s: LidarrStatus): boolean {
  return !s.notOwner && (((s.urlSet || s.awaySet) && s.hasKey) || !!s.homeSide);
}

/** Said where Get would be while Lidarr is set up at home but nothing reaches it from here. */
export const HOME_UNREACHABLE = "Getting music is paused: Platter can't reach your home right now. It comes back by itself.";

/**
 * The line a page shows where Get would be, or null: none while Lidarr is
 * ready or reachable (its check runs by itself, and Get shows when it
 * passes); a pause while home can't be reached; `setUp` only when there's
 * no Lidarr anywhere.
 */
export function lidarrNote(s: LidarrStatus | null, setUp: string): string | null {
  if (!s || s.ready || lidarrReachable(s)) return null;
  return s.homeQuiet ? HOME_UNREACHABLE : setUp;
}

/** [`lidarrNote`] for a page that asks for nothing else. */
export function useLidarrNote(setUp: string): string | null {
  return lidarrNote(useLidarrStatus(), setUp);
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
