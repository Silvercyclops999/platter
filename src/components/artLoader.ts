/**
 * How the phone loads covers: at most MAX_ART_LOADS at a time, covers on
 * screen before ones just past it, each fetched and decoded off screen
 * before it is shown, so a fast scroll never queues a pile of decodes and a
 * cover never draws half-finished. It also remembers which sizes of a cover
 * are already decoded, so a page can show a smaller one it has while the
 * right one comes (never a smaller one it would have to fetch).
 */

/** Covers loading at once, at most. */
export const MAX_ART_LOADS = 6;

const ready = new Set<string>();
const rungsByPath = new Map<string, Set<number>>();

/** Whether a cover URL is fetched and decoded already. */
export const isArtReady = (url: string | undefined): boolean => !!url && ready.has(url);

/** Note a decoded cover: its URL, and which size of which picture it is. */
export function markArtReady(url: string, path?: string | null, px?: number) {
  ready.add(url);
  if (!path || !px) return;
  const set = rungsByPath.get(path) ?? new Set<number>();
  set.add(px);
  rungsByPath.set(path, set);
}

/** The largest size of `path` already decoded that is smaller than `px`, if any. */
export function readyRungBelow(path: string | null | undefined, px: number): number | null {
  const set = path ? rungsByPath.get(path) : undefined;
  if (!set) return null;
  let best: number | null = null;
  for (const s of set) if (s < px && (best === null || s > best)) best = s;
  return best;
}

interface Job {
  url: string;
  visible: boolean;
  seq: number;
  cancelled: boolean;
  resolve: () => void;
  reject: (e: unknown) => void;
}

let running = 0;
let seq = 0;
const queue: Job[] = [];

/** Fetch and decode one image off screen. */
function fetchAndDecode(url: string): Promise<void> {
  if (typeof Image === "undefined") return Promise.reject(new Error("no images here"));
  const im = new Image();
  im.decoding = "async";
  const loaded = new Promise<void>((resolve, reject) => {
    im.onload = () => resolve();
    im.onerror = () => reject(new Error("art failed"));
  });
  im.src = url;
  return typeof im.decode === "function" ? im.decode().catch(() => loaded) : loaded;
}

function pump() {
  while (running < MAX_ART_LOADS && queue.length) {
    // On-screen covers first, then the oldest request.
    let at = queue.findIndex((j) => j.visible);
    if (at < 0) at = 0;
    const job = queue.splice(at, 1)[0];
    if (job.cancelled) continue;
    running++;
    fetchAndDecode(job.url).then(
      () => {
        ready.add(job.url);
        job.resolve();
      },
      (e) => job.reject(e),
    ).finally(() => {
      running--;
      pump();
    });
  }
}

export interface ArtLoad {
  done: Promise<void>;
  /** Drop it if it hasn't started (the cover scrolled away or unmounted). */
  cancel: () => void;
  /** It came on screen: load it ahead of covers that are only near. */
  promote: () => void;
}

/** Queue a cover. Resolves once it is decoded and can show at once. */
export function loadArt(url: string, opts: { visible: boolean }): ArtLoad {
  if (ready.has(url)) return { done: Promise.resolve(), cancel: () => {}, promote: () => {} };
  let job!: Job;
  const done = new Promise<void>((resolve, reject) => {
    job = { url, visible: opts.visible, seq: ++seq, cancelled: false, resolve, reject };
  });
  queue.push(job);
  pump();
  return {
    done,
    cancel: () => {
      job.cancelled = true;
      const i = queue.indexOf(job);
      if (i >= 0) queue.splice(i, 1);
    },
    promote: () => {
      job.visible = true;
    },
  };
}

/** What's loading and waiting (tests). */
export const artLoadState = () => ({ running, queued: queue.length });

/** Forget everything (tests). */
export function resetArtLoader() {
  ready.clear();
  rungsByPath.clear();
  queue.length = 0;
  running = 0;
}
