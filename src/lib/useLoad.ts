import { useCallback, useEffect, useRef, useState } from "react";
import { useApp } from "../stores/app";
import type { AppError } from "./types";

export interface LoadState<T> {
  data: T | undefined;
  error: AppError | undefined;
  loading: boolean;
}

export interface Loaded<T> extends LoadState<T> {
  /** Ask again (a Try again button), keeping what's shown meanwhile. */
  retry: () => void;
}

/**
 * Load data from the backend. Keeps the previous data while refetching (no
 * flash back to skeletons) and refetches when the library changes.
 */
export function useLoad<T>(fn: () => Promise<T>, deps: unknown[]): Loaded<T> {
  const libraryVersion = useApp((s) => s.libraryVersion);
  const [attempt, setAttempt] = useState(0);
  const retry = useCallback(() => setAttempt((n) => n + 1), []);
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
  }, [key, libraryVersion, attempt]);

  return { ...state, retry };
}

/**
 * `useLoad`'s data plus reloads asked for in the background (an event said
 * the page changed). Only the newest answer counts: a reload that finishes
 * after a later one, or after `useLoad` itself brought new data, is dropped,
 * and nothing is set once the page has gone.
 */
export function useReload<T>(loaded: T | undefined, fetch: () => Promise<T>): { data: T | undefined; reload: () => void } {
  const seq = useRef(0);
  const alive = useRef(true);
  const fetchRef = useRef(fetch);
  fetchRef.current = fetch;
  // A reload's answer counts only on top of the data it was asked over.
  const [fresh, setFresh] = useState<{ data: T; over: T | undefined } | null>(null);
  const base = useRef(loaded);
  base.current = loaded;
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  // New data from useLoad is the latest: reloads still on their way are dropped.
  useEffect(() => {
    seq.current++;
  }, [loaded]);
  const reload = useCallback(() => {
    const n = ++seq.current;
    const over = base.current;
    fetchRef.current().then(
      (data) => {
        if (alive.current && n === seq.current && over === base.current) setFresh({ data, over });
      },
      () => {},
    );
  }, []);
  return { data: fresh && fresh.over === loaded ? fresh.data : loaded, reload };
}
