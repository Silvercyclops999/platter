/**
 * The mix shelves for Home and Radio and mixes: read again when the mixes
 * are made again (`mixes://changed`) or the library changes, kept up while
 * the next answer loads, and a failure the page can show with Try again.
 * While only music on this computer can play, only mixes with downloaded
 * songs stay.
 */
import { useEffect, useState } from "react";
import { backend } from "../lib/backend";
import type { DownloadItem, MixShelves, MixSummary } from "../lib/types";
import { useApp } from "../stores/app";
import { useDownloads } from "../stores/downloads";

export interface MixShelvesState {
  /** Null while off or loading. */
  data: MixShelves | null;
  error: { message: string } | null;
  retry: () => void;
}

/** Mixes with at least one downloaded song (what can play offline). */
export function downloadedMixes(shelves: MixShelves, items: readonly DownloadItem[] | undefined): MixShelves {
  const ok = new Set((items ?? []).filter((i) => i.kind === "mix" && i.done > 0).map((i) => i.id));
  const keep = (list: MixSummary[]) => list.filter((m) => ok.has(m.id));
  return { madeForYou: keep(shelves.madeForYou), moods: keep(shelves.moods), genres: keep(shelves.genres) };
}

export function useMixShelves(on: boolean): MixShelvesState {
  const [version, setVersion] = useState(0);
  useEffect(() => {
    if (!on) return;
    let un: (() => void) | undefined;
    let alive = true;
    backend.listen("mixes://changed", () => setVersion((v) => v + 1)).then((u) => (alive ? (un = u) : u()));
    return () => {
      alive = false;
      un?.();
    };
  }, [on]);
  const libraryVersion = useApp((s) => s.libraryVersion);
  const [data, setData] = useState<MixShelves | null>(null);
  const [error, setError] = useState<{ message: string } | null>(null);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    if (!on) return;
    let alive = true;
    backend.mixesHome().then(
      (m) => {
        if (!alive) return;
        setData(m);
        setError(null);
      },
      (e: { message?: string }) => alive && setError({ message: e?.message ?? "Something went wrong." }),
    );
    return () => {
      alive = false;
    };
  }, [on, version, libraryVersion, attempt]);
  const cachedOnly = useApp((s) => !!s.health?.cachedOnly);
  const items = useDownloads((s) => s.status?.items);
  const shown = on && data ? (cachedOnly ? downloadedMixes(data, items) : data) : null;
  return { data: shown, error: on ? error : null, retry: () => setAttempt((n) => n + 1) };
}
