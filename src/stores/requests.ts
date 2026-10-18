/**
 * Lidarr requests as Get buttons show them. The core follows Lidarr's queue
 * and history every 15 seconds while something is in flight, and sends
 * waiting requests every few minutes; while a Get button that shows such a
 * request is on screen, this reads the core's list at the same pace (a
 * local read, nothing is asked of Lidarr from here).
 */
import { useEffect } from "react";
import { create } from "zustand";
import { backend } from "../lib/backend";
import type { LidarrRequest, LidarrRequestState } from "../lib/types";

/** How often the list is read again while a shown request is in flight. */
export const FOLLOW_MS = 15_000;
/** The first retry after a read that failed; it doubles up to {@link RETRY_MAX_MS}. */
export const RETRY_MS = 2_000;
export const RETRY_MAX_MS = 60_000;

/** Lidarr is still working on it. */
export function inFlight(s: LidarrRequestState | null | undefined): boolean {
  return s === "requested" || s === "searching" || s === "downloading" || s === "importing";
}

/** Worth reading again: Lidarr is working on it, or it's waiting to send. */
function followed(s: LidarrRequestState | null | undefined): boolean {
  return inFlight(s) || s === "queued";
}

interface RequestsState {
  list: LidarrRequest[];
  loaded: boolean;
}

export const useRequests = create<RequestsState>(() => ({ list: [], loaded: false }));

let watchers = 0;
let timer: ReturnType<typeof setTimeout> | null = null;
let loading: Promise<void> | null = null;
/** Failed reads in a row. */
let failures = 0;
/** Counts changes made here, so a read that started earlier doesn't undo them. */
let seq = 0;
const recent = new Map<string, { at: number; rec: LidarrRequest }>();

function schedule() {
  if (timer) clearTimeout(timer);
  timer = null;
  if (watchers <= 0) return;
  const st = useRequests.getState();
  if (!st.loaded) {
    // The first read failed: try again, a little later each time.
    if (failures > 0) timer = setTimeout(() => void refreshRequests(), Math.min(RETRY_MS * 2 ** (failures - 1), RETRY_MAX_MS));
    return;
  }
  if (st.list.some((r) => followed(r.state))) timer = setTimeout(() => void refreshRequests(), FOLLOW_MS);
}

/** The read's list, with anything changed here since it started on top. */
function merge(list: LidarrRequest[], since: number): LidarrRequest[] {
  const newer = [...recent.values()].filter((x) => x.at > since).map((x) => x.rec);
  for (const [id, x] of recent) if (x.at <= since) recent.delete(id);
  if (newer.length === 0) return list;
  const rg = (r: LidarrRequest) => r.releaseGroupId.toLowerCase();
  const rest = list.filter((x) => !newer.some((n) => n.id === x.id || rg(n) === rg(x)));
  return [...newer, ...rest];
}

/** Read the list again (shared by every button on screen). */
export function refreshRequests(): Promise<void> {
  if (loading) return loading;
  const since = seq;
  loading = backend
    .lidarrRequests(false)
    .then(
      (list) => {
        failures = 0;
        useRequests.setState({ list: merge(list, since), loaded: true });
      },
      () => {
        // Keep what's shown; a first read that failed is tried again.
        failures += 1;
      },
    )
    .finally(() => {
      loading = null;
      schedule();
    });
  return loading;
}

/** A request the core just made or changed. */
export function upsertRequest(r: LidarrRequest) {
  seq += 1;
  recent.set(r.id, { at: seq, rec: r });
  const list = useRequests.getState().list.filter((x) => x.id !== r.id && x.releaseGroupId !== r.releaseGroupId);
  useRequests.setState({ list: [r, ...list] });
  schedule();
}

/** Forget what was read (tests start clean). */
export function resetRequests() {
  if (timer) clearTimeout(timer);
  timer = null;
  failures = 0;
  recent.clear();
  useRequests.setState({ list: [], loaded: false });
}

/** The request for a release group, followed live while it's on screen. */
export function useRequestFor(releaseGroupId: string | null | undefined): LidarrRequest | null {
  const id = releaseGroupId?.toLowerCase() ?? "";
  const r = useRequests((s) => s.list.find((x) => x.releaseGroupId.toLowerCase() === id) ?? null);
  useEffect(() => {
    if (!id) return;
    watchers += 1;
    if (!useRequests.getState().loaded && !timer) void refreshRequests();
    else schedule();
    return () => {
      watchers -= 1;
      if (watchers === 0) schedule();
    };
  }, [id]);
  return r;
}
