/**
 * Settings > Lidarr > Home side: which of your own devices let this one use
 * Lidarr away from home (a computer at home with Lidarr set up, the
 * Platter Bridge), which of them answers first, and what to do when none
 * is on. Kept current from device sync and Bridge news.
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

export function LidarrHomeSides() {
  const h = useHomeSides();
  if (!h) return null;
  return (
    <div className="lidarr-home-sides" data-testid="lidarr-home-sides">
      {h.offering && <p className="lidarr-home-sides__line">Your other devices use Lidarr through this computer while Platter is open here.</p>}
      {h.sides.length > 0 ? (
        <ul className="lidarr-home-sides__list">
          {h.sides.map((s) => (
            <li key={`${s.kind}:${s.name}`} className="lidarr-home-sides__side" data-on={s.on ? "true" : "false"}>
              <span className="lidarr-home-sides__name">{s.name}</span>
              <span className="lidarr-home-sides__line">
                {s.words}
                {s.answers ? ". It answers for this device." : "."}
              </span>
            </li>
          ))}
        </ul>
      ) : (
        !h.offering && (
          <p className="lidarr-home-sides__line">None yet. A computer at home with Lidarr set up, or the Platter Bridge, lets this device use Lidarr away from home.</p>
        )
      )}
      {h.problem && (
        <p className="lidarr-check__error" role="alert">
          {h.problem}
        </p>
      )}
    </div>
  );
}
