/**
 * Sharing on the UI side: copying links (public lookups are slow, so the
 * clipboard write starts at the click and fills in when the link arrives),
 * and opening platter:// links the core hands over.
 *
 * Opening a song link shows its album. With a start time, the time is kept
 * until that song plays (from the toast's Play button or its row), then the
 * player seeks there once.
 */
import { joinCircle } from "./circle";
import { backend, type Backend } from "../lib/backend";
import { formatDuration } from "../lib/format";
import type { AppError, DeepLinkOpen, ShareKind } from "../lib/types";
import { player, usePlayer } from "./player";
import { nav } from "./router";
import { joinSession, listenAlong } from "./social";
import { openShareLink } from "./sharing";
import { toast, useToasts, type ToastArt } from "./toasts";

export const LINK_COPIED = "Link copied";

const errMessage = (e: unknown, fallback: string): string =>
  (e && typeof e === "object" && "message" in e ? String((e as AppError).message) : "") || fallback;

/**
 * Put `text` on the clipboard once it resolves. Where the browser allows it
 * the write starts now (inside the click) and waits for the text, so it
 * still counts as the user's action after a slow lookup.
 */
export function copyWhenReady(text: Promise<string>): Promise<void> {
  const clip = typeof navigator !== "undefined" ? navigator.clipboard : undefined;
  if (!clip) return Promise.reject(new Error("no clipboard"));
  if (typeof ClipboardItem !== "undefined" && typeof clip.write === "function") {
    const blob = text.then((t) => new Blob([t], { type: "text/plain" }));
    return clip.write([new ClipboardItem({ "text/plain": blob })]);
  }
  return text.then((t) => clip.writeText(t));
}

/** Copy a link (known now, or about to be). */
export function copyLink(text: string | Promise<string>, message = LINK_COPIED, art?: ToastArt): Promise<void> {
  return copyWhenReady(Promise.resolve(text)).then(
    () => void toast.info(message, art),
    () => void toast.error("Couldn't copy"),
  );
}

/**
 * Look up and copy the public link for a released song or album. Without a
 * sure match the toast offers the Platter link instead.
 */
export function copyPublicLink(kind: ShareKind, id: string, appLink: string, art?: ToastArt): Promise<void> {
  const link = backend.sharePublicLink(kind, id);
  const written = copyWhenReady(link);
  return link.then(
    () => written.then(
      () => void toast.info(LINK_COPIED, art),
      () => void toast.error("Couldn't copy"),
    ),
    (e: unknown) => {
      written.catch(() => {});
      const notFound = (e as AppError | undefined)?.kind === "notFound";
      useToasts.getState().push(
        {
          kind: notFound ? "info" : "error",
          message: errMessage(e, "Couldn't find a link"),
          action: { label: "Copy Platter link", run: () => void copyLink(appLink, LINK_COPIED, art) },
        },
        8000,
      );
    },
  );
}

/**
 * "Copy link at 1:23": the Platter link with the start time, after the
 * public link (with the time noted) when the song has one.
 */
export function copyLinksAt(kind: ShareKind, id: string, atSeconds: number, art?: ToastArt): Promise<void> {
  const at = formatDuration(atSeconds * 1000);
  let both = false;
  const text = backend.shareInfo(kind, id, atSeconds).then((info) =>
    info.public !== "available"
      ? info.appLink
      : backend.sharePublicLink(kind, id).then(
          (url) => {
            both = true;
            return `${url} (from ${at})\n${info.appLink}`;
          },
          () => info.appLink,
        ),
  );
  return copyWhenReady(text).then(
    () => void toast.info(both ? "Links copied" : LINK_COPIED, art),
    () => void toast.error("Couldn't copy"),
  );
}

// ---------------------------------------------------------------- opening links

/**
 * A start time waiting for its song to play. `armedOn` is the song that was
 * current when the link opened (something else starting drops the time);
 * `waitForRestart` is set when the linked song was already playing then, so
 * the time waits for the user to press play rather than jumping at once.
 */
let pendingStart: { trackId: string; ms: number; armedOn: string | null; waitForRestart: boolean } | null = null;

/** The start time waiting for a song, if any (tests). */
export const pendingStartFor = () => pendingStart;

/** Seek once when the song starts; forget it when something else plays. */
function onPlayer() {
  const p = pendingStart;
  if (!p) return;
  const s = usePlayer.getState().state;
  const now = s?.track?.id ?? null;
  if (now === p.trackId) {
    if (s?.status !== "playing") p.waitForRestart = false;
    else if (!p.waitForRestart) {
      pendingStart = null;
      void player.seek(p.ms);
    }
  } else if (now !== p.armedOn) {
    pendingStart = null;
  }
}

function armStart(trackId: string, ms: number) {
  const s = usePlayer.getState().state;
  const current = s?.track?.id ?? null;
  pendingStart = {
    trackId,
    ms,
    armedOn: current === trackId ? null : current,
    waitForRestart: current === trackId && s?.status === "playing",
  };
  onPlayer();
}

async function songTitle(albumId: string, trackId: string, b: Backend): Promise<string | null> {
  try {
    return (await b.albumPage(albumId)).tracks.find((t) => t.id === trackId)?.title ?? null;
  } catch {
    return null;
  }
}

/** Show what a link points at. */
export async function openDeepLink(o: DeepLinkOpen, b: Backend = backend): Promise<void> {
  if (o.circle) {
    await joinCircle(o.circle, b);
    return;
  }
  if (o.join) {
    if (!listenAlong.enabled) {
      toast.info("Listening sessions are coming back in an update soon.");
      return;
    }
    await joinSession(o.join, b);
    return;
  }
  if (o.shareAsk) {
    await openShareLink(o.shareAsk, b);
    return;
  }
  if (o.status === "notInLibrary") {
    toast.info("This isn't in your library");
    return;
  }
  if (o.status !== "open" || !o.kind || !o.id) {
    toast.error("Platter couldn't open that link");
    return;
  }
  const id = o.id;
  if (o.kind !== "track") {
    nav.go({ name: o.kind, id });
    return;
  }
  if (!o.albumId) return;
  const albumId = o.albumId;
  nav.go({ name: "album", id: albumId });
  const title = (await songTitle(albumId, id, b)) ?? "the song";
  const named = title === "the song" ? title : `"${title}"`;
  const ms = (o.atSeconds ?? 0) * 1000;
  if (ms > 0) armStart(id, ms);
  useToasts.getState().push(
    {
      kind: "info",
      message: ms > 0 ? `Play ${named} to start at ${formatDuration(ms)}` : `Shared song: ${named}`,
      action: { label: "Play", run: () => void player.playAlbum(albumId, id) },
    },
    10000,
  );
}

/** Take a waiting link from the core and open it. */
async function takeAndOpen(b: Backend) {
  try {
    const o = await b.deepLinkTake();
    if (o) await openDeepLink(o, b);
  } catch {
    /* nothing waiting */
  }
}

/** Open links that started the app or arrive later. Returns a cleanup. */
export function connectShare(b: Backend = backend): () => void {
  let alive = true;
  let unlisten: (() => void) | null = null;
  void takeAndOpen(b);
  b.listen("deeplink://open", () => void takeAndOpen(b)).then((u) => (alive ? (unlisten = u) : u()));
  const unsub = usePlayer.subscribe(onPlayer);
  return () => {
    alive = false;
    unlisten?.();
    unsub();
    pendingStart = null;
  };
}
