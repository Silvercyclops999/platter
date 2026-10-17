/**
 * The small, always-loaded side of Lidarr requests: whether Request shows,
 * an artist's missing releases, and request state names. The cards and the
 * request dialog live in LidarrRequest.tsx, loaded when first needed.
 */
import { useCallback, useEffect, useState } from "react";
import { backend } from "../lib/backend";
import type { LidarrMissing, LidarrMissingItem, LidarrRequestState } from "../lib/types";
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
    if (!artist || !on || !(home || away)) {
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

/** The missing releases a discography filter shows: albums under Albums, EPs under Singles and EPs. */
export function missingFor(items: LidarrMissingItem[], filter: string): LidarrMissingItem[] {
  if (filter === "popular" || filter === "albums") return items.filter((i) => i.kind === "album");
  if (filter === "singlesEps") return items.filter((i) => i.kind === "ep");
  return [];
}

/**
 * True when Request should show: Lidarr is set up with a key and its last
 * setup check passed. Every request still runs the check again first.
 */
export function useLidarrReady(): boolean {
  const home = useSettings((s) => s.settings?.lidarrUrl);
  const away = useSettings((s) => s.settings?.lidarrAwayUrl);
  const [ready, setReady] = useState(false);
  useEffect(() => {
    if (!(home || away)) {
      setReady(false);
      return;
    }
    let alive = true;
    backend.lidarrStatus().then(
      (s) => alive && setReady(s.ready),
      () => alive && setReady(false),
    );
    return () => {
      alive = false;
    };
  }, [home, away]);
  return ready;
}
