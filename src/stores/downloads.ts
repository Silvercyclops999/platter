/**
 * Downloads as the UI sees them: the core's status (items, progress, the
 * queue's state, storage), kept current from `downloads://status`, and the
 * set of downloaded songs (rows show a mark), reloaded on `downloads://changed`.
 * Removing a download always asks first. An item that finishes says
 * "Download complete". A downloaded item that gains songs (a playlist edit,
 * a new like) catches up by itself and says "Downloading N songs".
 */
import { create } from "zustand";
import { backend, type Backend } from "../lib/backend";
import { formatBytes } from "../lib/format";
import type { AppError, DownloadItem, DownloadKind, DownloadPause, DownloadStatus } from "../lib/types";
import type { MenuItem } from "./ui";
import { toast } from "./toasts";

export interface DownloadTarget {
  kind: DownloadKind;
  /** Ignored for Liked Songs. */
  id: string;
  title: string;
  /** A mix's songs (albums, playlists and Liked Songs come from the library). */
  trackIds?: () => string[];
}

interface DownloadsState {
  status: DownloadStatus | null;
  ids: ReadonlySet<string>;
  /** A removal waiting for the user to confirm it. */
  confirming: { target: DownloadTarget; bytes: number } | null;
}

export const useDownloads = create<DownloadsState>()(() => ({ status: null, ids: new Set(), confirming: null }));

let started: Backend | null = null;
let unsubs: Array<() => void> = [];

const reloadIds = (b: Backend) =>
  b.downloadedTracks().then(
    (ids) => useDownloads.setState({ ids: new Set(ids) }),
    () => {},
  );

/** Load the status and follow its events (once, at startup). */
export async function startDownloads(b: Backend = backend): Promise<void> {
  if (started === b) return;
  started = b;
  unsubs.push(await b.listen("downloads://status", (status) => takeStatus(status)));
  unsubs.push(await b.listen("downloads://changed", () => void reloadIds(b)));
  await Promise.all([
    b.downloadsStatus().then(
      (status) => useDownloads.setState({ status }),
      () => {},
    ),
    reloadIds(b),
  ]);
}

/** "Downloading 3 songs". */
export const catchingUpMessage = (n: number) => `Downloading ${n} ${n === 1 ? "song" : "songs"}`;

/** A new status; items that just finished, or started catching up, say so. */
function takeStatus(status: DownloadStatus) {
  const before = useDownloads.getState().status;
  useDownloads.setState({ status });
  if (!before) return;
  for (const it of status.items) {
    const was = findItem(before, it.kind, it.id);
    if (!was) continue;
    if (it.state === "downloaded" && was.state !== "downloaded" && it.total > 0) toast.info("Download complete");
    // New songs on a downloaded item, fetching now (not held by Go offline or the like).
    const catchingUp = was.state === "downloaded" && it.total > was.total && it.state !== "downloaded" && it.state !== "error";
    if (catchingUp && !status.paused) toast.info(catchingUpMessage(it.total - it.done));
  }
}

/** For tests: forget the subscription and state. */
export function resetDownloads() {
  unsubs.forEach((u) => u());
  unsubs = [];
  started = null;
  useDownloads.setState({ status: null, ids: new Set(), confirming: null });
}

const sameId = (kind: DownloadKind, a: string, b: string) => kind === "liked" || a === b;

export function findItem(s: DownloadStatus | null, kind: DownloadKind, id: string): DownloadItem | undefined {
  return s?.items.find((i) => i.kind === kind && sameId(kind, i.id, id));
}

/** The item's download, or undefined when it isn't downloaded. */
export function useDownloadItem(kind: DownloadKind, id: string): DownloadItem | undefined {
  return useDownloads((s) => findItem(s.status, kind, id));
}

export function useIsDownloaded(trackId: string): boolean {
  return useDownloads((s) => s.ids.has(trackId));
}

const message = (e: unknown) => (e && typeof e === "object" && "message" in e ? String((e as AppError).message) : "Something went wrong.");

/** Turn a download on or off. */
export async function setDownload(t: DownloadTarget, on: boolean): Promise<boolean> {
  try {
    const status = await backend.downloadsSet(t.kind, t.id, on, { title: t.title, trackIds: t.trackIds?.() });
    takeStatus(status);
    return true;
  } catch (e) {
    toast.error(`Couldn't ${on ? "download" : "remove"} ${t.title}: ${message(e)}`);
    return false;
  }
}

/** The download button and menu item: download, or remove (asking first). */
export function toggleDownload(t: DownloadTarget) {
  const item = findItem(useDownloads.getState().status, t.kind, t.id);
  if (!item) return void setDownload(t, true);
  useDownloads.setState({ confirming: { target: t, bytes: item.bytes } });
}

export function answerRemove(remove: boolean) {
  const ask = useDownloads.getState().confirming;
  useDownloads.setState({ confirming: null });
  if (ask && remove) void setDownload(ask.target, false);
}

/** "Download" or "Remove download" for a menu. Error items offer to try again. */
export function downloadMenuItems(t: DownloadTarget): MenuItem[] {
  const item = () => findItem(useDownloads.getState().status, t.kind, t.id);
  return [
    {
      // Read when the menu shows.
      get label() {
        return item() ? "Remove download" : "Download";
      },
      onSelect: () => toggleDownload(t),
    },
    {
      label: "Download again",
      when: () => {
        const i = item();
        return !!i && i.failed + i.missing > 0;
      },
      onSelect: () => void retryDownload(t.kind, t.id),
    },
  ];
}

export async function retryDownload(kind?: DownloadKind, id?: string) {
  try {
    await backend.downloadsRetry(kind, id);
  } catch (e) {
    toast.error(message(e));
  }
}

export async function removeAllDownloads(): Promise<void> {
  try {
    const freed = await backend.downloadsRemoveAll();
    toast.info(freed > 0 ? `Removed every download and freed ${formatBytes(freed)}` : "There were no downloads to remove");
  } catch (e) {
    toast.error(`Couldn't remove downloads: ${message(e)}`);
  }
}

export async function chooseDownloadFolder(): Promise<void> {
  try {
    const folder = await backend.downloadsChooseFolder();
    if (folder) toast.info("Downloads moved");
  } catch (e) {
    toast.error(`Couldn't move downloads: ${message(e)}`);
  }
}

/** What the waiting queue says. */
export function pauseText(p: DownloadPause | null | undefined): string | null {
  switch (p) {
    case "offline":
      return "Waiting for your server.";
    case "notHome":
      return "Waiting for your home network.";
    case "full":
      return "Stopped: your downloads reached the storage limit.";
    case "noSpace":
      return "Stopped: this computer is out of space.";
    default:
      return null;
  }
}

/** The tooltip and accessible state of an item: "Downloaded", "Downloading, 40%". */
export function itemText(item: DownloadItem | undefined, paused?: DownloadPause | null): string {
  if (!item) return "Download";
  switch (item.state) {
    case "downloaded":
      return "Downloaded";
    case "error":
      return item.missing > 0 && item.failed === 0 ? "Some songs are missing from this computer" : "Some songs couldn't download";
    case "downloading":
      return `Downloading, ${Math.round(item.progress * 100)}%`;
    default:
      return pauseText(paused) ?? "Waiting to download";
  }
}
