/**
 * Settings > Lidarr: a line when Lidarr can't be reached away from home
 * (no computer at home or Bridge answers), with what's wrong. Kept current
 * from device sync and Bridge news.
 */
import { useCallback, useEffect, useState } from "react";
import { backend } from "../lib/backend";
import type { HomeSides } from "../lib/types";
import { useSettings } from "../stores/settings";
import "./lidarr-settings.css";

/** Read again whenever a linked device or a Bridge says something new,
 * and when this computer's own setting or Lidarr address changes. */
export function useHomeSides(): HomeSides | null {
  const [sides, setSides] = useState<HomeSides | null>(null);
  const on = useSettings((st) => st.settings?.homeSideLidarr);
  const home = useSettings((st) => st.settings?.lidarrUrl);
  const away = useSettings((st) => st.settings?.lidarrAwayUrl);
  const load = useCallback(() => {
    backend.lidarrHomeSides().then(setSides, () => {});
  }, []);
  useEffect(() => {
    load();
  }, [load, on, home, away]);
  useEffect(() => {
    const uns = [backend.listen("device-sync://status", load), backend.listen("bridge://changed", load)];
    return () => uns.forEach((un) => void un.then((f) => f()));
  }, [load]);
  return sides;
}

/** What's wrong away from home, and only then (the row is hidden otherwise). */
export function LidarrAwayProblem({ sides }: { sides: HomeSides | null }) {
  if (!sides?.problem) return null;
  return (
    <p className="lidarr-check__error" role="alert" data-testid="lidarr-away-problem">
      {sides.problem}
    </p>
  );
}
