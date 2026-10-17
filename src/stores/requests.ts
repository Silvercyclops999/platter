/**
 * Lidarr requests as Get buttons show them. The core follows Lidarr's queue
 * and history every 15 seconds while something is in flight; while a Get
 * button that shows such a request is on screen, this reads the core's list
 * at the same pace (a local read, nothing is asked of Lidarr from here).
 */
import { useEffect } from "react";
import { create } from "zustand";
import { backend } from "../lib/backend";
import type { LidarrRequest, LidarrRequestState } from "../lib/types";

/** How often the list is read again while a shown request is in flight. */
export const FOLLOW_MS = 15_000;

/** Lidarr is still working on it. */
export function inFlight(s: LidarrRequestState | null | undefined): boolean {
  return s === "requested" || s === "searching" || s === "downloading" || s === "importing";
}

interface RequestsState {
  list: LidarrRequest[];
  loaded: boolean;
}

export const useRequests = create<RequestsState>(() => ({ list: [], loaded: false }));

let watchers = 0;
let timer: ReturnType<typeof setTimeout> | null = null;
let loading: Promise<void> | null = null;

function schedule() {
  if (timer) clearTimeout(timer);
  timer = null;
  if (watchers > 0 && useRequests.getState().list.some((r) => inFlight(r.state))) {
    timer = setTimeout(() => void refreshRequests(), FOLLOW_MS);
  }
}

/** Read the list again (shared by every button on screen). */
export function refreshRequests(): Promise<void> {
  if (loading) return loading;
  loading = backend
    .lidarrRequests(false)
    .then(
      (list) => useRequests.setState({ list, loaded: true }),
      () => useRequests.setState({ loaded: true }),
    )
    .finally(() => {
      loading = null;
      schedule();
    });
  return loading;
}

/** A request the core just made or changed. */
export function upsertRequest(r: LidarrRequest) {
  const list = useRequests.getState().list.filter((x) => x.id !== r.id && x.releaseGroupId !== r.releaseGroupId);
  useRequests.setState({ list: [r, ...list] });
  schedule();
}

/** Forget what was read (tests start clean). */
export function resetRequests() {
  if (timer) clearTimeout(timer);
  timer = null;
  useRequests.setState({ list: [], loaded: false });
}

/** The request for a release group, followed live while it's on screen. */
export function useRequestFor(releaseGroupId: string | null | undefined): LidarrRequest | null {
  const id = releaseGroupId?.toLowerCase() ?? "";
  const r = useRequests((s) => s.list.find((x) => x.releaseGroupId.toLowerCase() === id) ?? null);
  useEffect(() => {
    if (!id) return;
    watchers += 1;
    if (!useRequests.getState().loaded) void refreshRequests();
    else schedule();
    return () => {
      watchers -= 1;
      if (watchers === 0) schedule();
    };
  }, [id]);
  return r;
}
