/**
 * The release radar as the UI sees it: the page (Home's shelf reads the same
 * list), read again on `radar://changed`.
 */
import { useEffect, useState } from "react";
import { backend } from "../lib/backend";
import type { RadarPage } from "../lib/types";
import { useSettings } from "./settings";

/** The radar page; null while it loads (or when it can't be read). Read again when it changes. */
export function useRadar(): { page: RadarPage | null; reload: () => void } {
  const on = useSettings((s) => s.settings?.releaseRadar !== false);
  const home = useSettings((s) => s.settings?.lidarrUrl);
  const [version, setVersion] = useState(0);
  const [page, setPage] = useState<RadarPage | null>(null);
  useEffect(() => {
    let un: (() => void) | undefined;
    let alive = true;
    backend.listen("radar://changed", () => setVersion((v) => v + 1)).then((u) => (alive ? (un = u) : u()));
    return () => {
      alive = false;
      un?.();
    };
  }, []);
  useEffect(() => {
    let alive = true;
    backend.releaseRadar().then(
      (p) => alive && setPage(p),
      () => alive && setPage(null),
    );
    return () => {
      alive = false;
    };
  }, [on, home, version]);
  return { page, reload: () => setVersion((v) => v + 1) };
}
