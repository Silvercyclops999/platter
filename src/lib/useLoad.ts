import { useEffect, useRef, useState } from "react";
import { useApp } from "../stores/app";
import type { AppError } from "./types";

export interface LoadState<T> {
  data: T | undefined;
  error: AppError | undefined;
  loading: boolean;
}

/**
 * Load data from the backend. Keeps the previous data while refetching (no
 * flash back to skeletons) and refetches when the library changes.
 */
export function useLoad<T>(fn: () => Promise<T>, deps: unknown[]): LoadState<T> {
  const libraryVersion = useApp((s) => s.libraryVersion);
  const [state, setState] = useState<LoadState<T>>({ data: undefined, error: undefined, loading: true });
  const keyRef = useRef<string>("");
  const key = JSON.stringify(deps);

  useEffect(() => {
    let alive = true;
    const keyChanged = keyRef.current !== key;
    keyRef.current = key;
    // New target → drop old data so the skeleton shows; library refresh → keep it.
    setState((s) => (keyChanged ? { data: undefined, error: undefined, loading: true } : { ...s, loading: true }));
    fn().then(
      (data) => alive && setState({ data, error: undefined, loading: false }),
      (error: AppError) => alive && setState((s) => ({ data: keyChanged ? undefined : s.data, error, loading: false })),
    );
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, libraryVersion]);

  return state;
}
