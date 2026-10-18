import { useEffect, useState } from "react";
import { backend } from "../lib/backend";
import type { CachedLibrary } from "../lib/types";
import { useApp } from "../stores/app";

/**
 * While only music on this computer can play, what that covers (refetched
 * as the cached list changes). `lib` is null otherwise, while it loads, or
 * when it couldn't be read (`failed`: not the same as nothing stored).
 */
export function useCachedLibrary(): { lib: CachedLibrary | null; failed: boolean; retry: () => void } {
  const cachedOnly = useApp((s) => !!s.health?.cachedOnly);
  const cached = useApp((s) => s.cached);
  const [lib, setLib] = useState<CachedLibrary | null>(null);
  const [failed, setFailed] = useState(false);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    if (!cachedOnly) {
      setLib(null);
      setFailed(false);
      return;
    }
    let alive = true;
    backend.cachedLibrary().then(
      (l) => {
        if (!alive) return;
        setLib(l);
        setFailed(false);
      },
      // The last answer stays if there was one.
      () => alive && setFailed(true),
    );
    return () => {
      alive = false;
    };
  }, [cachedOnly, cached, attempt]);
  return { lib: cachedOnly ? lib : null, failed: cachedOnly && failed && !lib, retry: () => setAttempt((n) => n + 1) };
}
