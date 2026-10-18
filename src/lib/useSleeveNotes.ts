import { useEffect, useState } from "react";
import { useSettings } from "../stores/settings";
import { backend } from "./backend";
import type { SleeveNotes, SongCredits } from "./types";

export interface Loaded<T> {
  data: T | null;
  /** The first answer hasn't come yet. */
  loading: boolean;
  failed: boolean;
}

/**
 * An album's sleeve notes, asked for only while `on` (the tab is open).
 * Answers from the core's cache at once and asks again when
 * `sleeve://changed` says lookups finished.
 */
export function useSleeveNotes(albumId: string | null | undefined, on = true): Loaded<SleeveNotes> {
  const [state, setState] = useState<Loaded<SleeveNotes>>({ data: null, loading: true, failed: false });
  const show = useSettings((s) => s.settings?.showSleeveNotes !== false);
  const lookups = useSettings((s) => s.settings?.sleeveLookups);
  useEffect(() => {
    setState({ data: null, loading: true, failed: false });
    if (!albumId || !on || !show) return;
    let alive = true;
    const load = () =>
      backend.sleeveNotes(albumId).then(
        (data) => alive && setState({ data, loading: false, failed: false }),
        () => alive && setState((s) => ({ data: s.data, loading: false, failed: !s.data })),
      );
    void load();
    const un = backend.listen("sleeve://changed", (e) => {
      if (e.id === albumId) void load();
    });
    return () => {
      alive = false;
      un.then((f) => f());
    };
  }, [albumId, on, show, lookups]);
  return state;
}

/** One song's credits, for the Credits dialog. */
export function useSongCredits(trackId: string | null): Loaded<SongCredits> {
  const [state, setState] = useState<Loaded<SongCredits>>({ data: null, loading: true, failed: false });
  useEffect(() => {
    setState({ data: null, loading: true, failed: false });
    if (!trackId) return;
    let alive = true;
    let albumId: string | null = null;
    const load = () =>
      backend.songCredits(trackId).then(
        (data) => {
          albumId = data.albumId;
          if (alive) setState({ data, loading: false, failed: false });
        },
        () => alive && setState((s) => ({ data: s.data, loading: false, failed: !s.data })),
      );
    void load();
    const un = backend.listen("sleeve://changed", (e) => {
      if (e.id === albumId) void load();
    });
    return () => {
      alive = false;
      un.then((f) => f());
    };
  }, [trackId]);
  return state;
}
