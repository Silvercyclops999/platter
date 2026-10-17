/**
 * Lyrics transcription as the UI sees it: the core's status (models, the
 * song being worked on, the queue, the last failure), kept current from
 * `transcribe://status`, and the songs found to have no words. A finished
 * song bumps the lyrics editor's version so the lyrics views ask again.
 */
import { create } from "zustand";
import { backend } from "../lib/backend";
import type { AppError, TranscribeModel, TranscribeStatus } from "../lib/types";
import { ownLyricsChanged } from "./lyricsEditor";
import { toast } from "./toasts";

interface TranscribeState {
  status: TranscribeStatus | null;
  /** Songs transcribed with no words found (learned this session). */
  empty: ReadonlySet<string>;
}

export const useTranscribe = create<TranscribeState>()(() => ({ status: null, empty: new Set() }));

let started = false;

/** Load the status and follow its events (once, at startup). */
export async function startTranscribe(): Promise<void> {
  if (started) return;
  started = true;
  await backend.listen("transcribe://status", (status) => useTranscribe.setState({ status }));
  await backend.listen("transcribe://done", ({ trackId, found }) => {
    if (found) ownLyricsChanged(trackId, true);
    else useTranscribe.setState((s) => ({ empty: new Set(s.empty).add(trackId) }));
  });
  try {
    useTranscribe.setState({ status: await backend.transcribeStatus() });
  } catch {
    // The rows show as unavailable until a status event arrives.
  }
}

/** For tests: forget the subscription. */
export function resetTranscribe() {
  started = false;
  useTranscribe.setState({ status: null, empty: new Set() });
}

const message = (e: unknown) => (e && typeof e === "object" && "message" in e ? String((e as AppError).message) : "Something went wrong.");

async function run<T>(f: () => Promise<T>): Promise<T | undefined> {
  try {
    return await f();
  } catch (e) {
    toast.error(message(e));
    return undefined;
  }
}

export const transcribe = {
  track: (trackId: string) => run(() => backend.transcribeTrack(trackId)),
  album: async (albumId: string) => {
    const n = await run(() => backend.transcribeAlbum(albumId));
    if (n === 0) toast.info("Every song on this album already has lyrics.");
    else if (n) toast.info(n === 1 ? "Transcribing 1 song" : `Transcribing ${n} songs`);
  },
  unreleased: async () => {
    const n = await run(() => backend.transcribeUnreleased());
    if (n === 0) toast.info("No unreleased songs are waiting for lyrics.");
  },
  cancel: (trackId?: string) => run(() => backend.transcribeCancel(trackId)),
  pause: () => run(() => backend.transcribePause(true)),
  resume: () => run(() => backend.transcribePause(false)),
  redo: async () => {
    const n = await run(() => backend.transcribeRedo());
    if (n) toast.info(n === 1 ? "Transcribing 1 song again" : `Transcribing ${n} songs again`);
  },
  download: (model: TranscribeModel) => run(() => backend.transcribeModelDownload(model)),
  cancelDownload: () => run(() => backend.transcribeModelCancel()),
  deleteModel: (model: TranscribeModel) => run(() => backend.transcribeModelDelete(model)),
};

/** Whether the song was transcribed and had no words (asks the core once). */
export async function checkFoundNothing(trackId: string): Promise<void> {
  if (useTranscribe.getState().empty.has(trackId)) return;
  try {
    if (await backend.transcribeFoundNothing(trackId)) useTranscribe.setState((s) => ({ empty: new Set(s.empty).add(trackId) }));
  } catch {
    // Unknown: the view offers to transcribe.
  }
}

/** "190 MB", "1.2 GB". */
export function modelSize(bytes: number): string {
  const mb = bytes / (1024 * 1024);
  return mb >= 1000 ? `${(mb / 1024).toFixed(1)} GB` : `${Math.round(mb)} MB`;
}
