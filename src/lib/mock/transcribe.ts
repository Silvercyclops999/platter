/**
 * Mock of lyrics transcription (docs/IPC.md "Lyrics transcription"): a fake
 * model download with progress, and a canned transcription saved as the
 * song's own lyrics marked auto. Test hooks fail the next download (network,
 * checksum or disk) or the next song.
 */
import type { Settings, TranscribeModel, TranscribeStatus } from "../types";

export interface MockTranscribeHooks {
  settings: () => Settings;
  emitStatus: (s: TranscribeStatus) => void;
  emitDone: (trackId: string, found: boolean) => void;
  later: (ms: number, fn: () => void) => void;
  /** Lyrics from somewhere other than transcription. */
  hasLyrics: (trackId: string) => boolean;
  /** Saves the canned result as the song's own lyrics, marked auto. */
  save: (trackId: string, text: string, model: TranscribeModel) => void;
  albumTracks: (albumId: string) => string[];
  unreleasedTracks: () => string[];
}

const SIZES: Record<TranscribeModel, number> = { small: 190_098_681, medium: 539_225_533 };

/** Invented words, the same for every song. */
export const CANNED_LRC = [
  "[00:08.00]Somewhere the night is still awake",
  "[00:12.40]I walk the long way back to you",
  "[00:17.10]Every light along the road",
  "[00:21.60]Says the morning's coming through",
].join("\n");

type Fail = "download" | "checksum" | "disk" | null;

export class MockTranscribe {
  /** Step length for fake progress (tests make it short). */
  stepMs = 150;
  /** Test hooks. */
  failNextDownload: Fail = null;
  failNextSong = false;
  /** Songs where "no words" come out. */
  silent = new Set<string>();
  /** Songs transcribed with no words found. */
  empty = new Set<string>();
  /** The computer is busy: background songs wait. */
  busy = false;
  /** Seconds per song the status reports for a long run. */
  secsPerSong: number | undefined = undefined;

  private installed = new Set<TranscribeModel>();
  private download: { kind: TranscribeModel; done: number; cancelled: boolean } | null = null;
  private queue: { trackId: string; background: boolean }[] = [];
  private current: TranscribeStatus["current"];
  private batch = { done: 0, total: 0 };
  private error: TranscribeStatus["error"];
  private running = false;
  /** Bumped by reset, so a job still stepping from before stops. */
  private generation = 0;

  constructor(private hooks: MockTranscribeHooks) {}

  /** Back to a fresh install (tests). */
  reset() {
    this.generation++;
    this.installed.clear();
    this.download = null;
    this.queue = [];
    this.current = undefined;
    this.batch = { done: 0, total: 0 };
    this.error = undefined;
    this.running = false;
    this.failNextDownload = null;
    this.failNextSong = false;
    this.silent.clear();
    this.empty.clear();
    this.busy = false;
    this.secsPerSong = undefined;
  }

  status(): TranscribeStatus {
    const s = this.hooks.settings();
    return {
      enabled: s.transcribeLyrics,
      available: true,
      model: s.transcribeModel,
      models: (["small", "medium"] as const).map((kind) => ({
        kind,
        sizeBytes: SIZES[kind],
        installed: this.installed.has(kind),
        downloading: this.download?.kind === kind,
        doneBytes: this.download?.kind === kind ? this.download.done : 0,
      })),
      current: this.current ? { ...this.current } : undefined,
      queued: this.queue.length,
      upNext: this.queue[0]?.trackId,
      batch: { ...this.batch },
      secsPerSong: this.batch.total > 1 ? this.secsPerSong : undefined,
      paused: this.busy && this.batch.total > 0 ? "busy" : undefined,
      error: this.error,
    };
  }

  private emit() {
    this.hooks.emitStatus(this.status());
  }

  /** Settings changed: turning it off stops everything. */
  settingsChanged() {
    if (!this.hooks.settings().transcribeLyrics) {
      this.queue = [];
      this.current = undefined;
      this.batch = { done: 0, total: 0 };
      if (this.download) this.download.cancelled = true;
    }
    this.emit();
  }

  private refuse(message: string): never {
    throw { kind: "invalid", message };
  }

  downloadModel(kind: TranscribeModel) {
    if (this.installed.has(kind)) return;
    if (this.download) this.refuse("A model is already downloading.");
    this.error = undefined;
    const fail = this.failNextDownload;
    this.failNextDownload = null;
    if (fail === "disk") {
      this.error = { kind: "disk", message: `Not enough free space. The model needs ${Math.ceil(SIZES[kind] / 1048576) + 256} MB free and 120 MB is available.` };
      this.emit();
      return;
    }
    const dl = { kind, done: 0, cancelled: false };
    this.download = dl;
    this.emit();
    const step = (n: number) => {
      if (this.download !== dl) return;
      if (dl.cancelled) {
        this.download = null;
        this.emit();
        return;
      }
      dl.done = Math.round((SIZES[kind] * n) / 10);
      if (n === 5 && fail === "download") return this.downloadFailed("download", "The model download failed. Check your connection and try again.");
      if (n < 10) {
        this.emit();
        this.hooks.later(this.stepMs, () => step(n + 1));
        return;
      }
      if (fail === "checksum") return this.downloadFailed("checksum", "The downloaded model didn't match its checksum, so it was deleted. Try again.");
      this.download = null;
      this.installed.add(kind);
      this.emit();
      this.pump();
    };
    this.hooks.later(this.stepMs, () => step(1));
  }

  private downloadFailed(kind: "download" | "checksum", message: string) {
    this.download = null;
    this.error = { kind, message };
    if (this.current?.stage === "waitingForModel") {
      this.current = undefined;
      this.queue = [];
      this.batch = { done: 0, total: 0 };
    }
    this.emit();
  }

  cancelDownload() {
    if (this.download) this.download.cancelled = true;
  }

  deleteModel(kind: TranscribeModel) {
    if (this.download?.kind === kind) this.refuse("Cancel the download first.");
    this.installed.delete(kind);
    this.emit();
  }

  private usable() {
    if (!this.hooks.settings().transcribeLyrics) this.refuse("Turn on transcription in Settings first.");
  }

  private enqueue(ids: string[], background: boolean, front: boolean): number {
    this.usable();
    const model = this.hooks.settings().transcribeModel;
    if (!this.installed.has(model) && !this.download) this.downloadModel(model);
    const fresh = ids.filter((id) => this.current?.trackId !== id && !this.queue.some((j) => j.trackId === id));
    const jobs = fresh.map((trackId) => ({ trackId, background }));
    if (front) this.queue.unshift(...jobs);
    else if (background) this.queue.push(...jobs);
    else {
      const at = this.queue.findIndex((j) => j.background);
      this.queue.splice(at < 0 ? this.queue.length : at, 0, ...jobs);
    }
    this.batch.total += jobs.length;
    this.error = undefined;
    this.emit();
    this.pump();
    return jobs.length;
  }

  transcribeTrack(trackId: string) {
    this.enqueue([trackId], false, true);
  }
  transcribeAlbum(albumId: string) {
    this.usable();
    return this.enqueue(this.hooks.albumTracks(albumId).filter((id) => !this.empty.has(id)), false, false);
  }
  transcribeUnreleased() {
    this.usable();
    return this.enqueue(this.hooks.unreleasedTracks().filter((id) => !this.empty.has(id)), true, false);
  }

  cancel(trackId?: string) {
    if (!trackId) {
      this.queue = [];
      this.current = undefined;
      this.batch = { done: 0, total: 0 };
    } else {
      this.queue = this.queue.filter((j) => j.trackId !== trackId);
      if (this.current?.trackId === trackId) this.current = undefined;
    }
    this.emit();
    this.pump();
  }

  private pump() {
    if (this.running) return;
    const model = this.hooks.settings().transcribeModel;
    let job: { trackId: string; background: boolean } | undefined;
    if (this.current?.stage === "waitingForModel") {
      if (!this.installed.has(model)) return;
      job = { trackId: this.current.trackId, background: this.current.background };
    } else if (this.current) return;
    else job = this.queue.shift();
    if (!job) {
      this.batch = { done: 0, total: 0 };
      this.emit();
      return;
    }
    this.current = { trackId: job.trackId, stage: "reading", percent: 0, background: job.background };
    if (!this.installed.has(model)) {
      this.current.stage = "waitingForModel";
      this.emit();
      return; // the download's end pumps again
    }
    this.running = true;
    this.emit();
    const id = job.trackId;
    const generation = this.generation;
    const step = (pct: number) => {
      // Reset meanwhile (the next test): this job is gone.
      if (generation !== this.generation) return;
      if (this.current?.trackId !== id) {
        this.running = false;
        this.pump();
        return;
      }
      if (pct > 100) return this.finish(job.trackId, job.background);
      this.current = { ...this.current, stage: "transcribing", percent: pct };
      this.emit();
      this.hooks.later(this.stepMs, () => step(pct + 25));
    };
    this.hooks.later(this.stepMs, () => step(0));
  }

  private finish(trackId: string, background: boolean) {
    this.running = false;
    this.current = undefined;
    this.batch.done = Math.min(this.batch.total, this.batch.done + 1);
    const skip = background && this.hooks.hasLyrics(trackId);
    if (this.failNextSong) {
      this.failNextSong = false;
      this.error = { kind: "transcribe", message: "Transcription failed for this song.", trackId };
    } else if (!skip) {
      const found = !this.silent.has(trackId);
      if (found) this.hooks.save(trackId, CANNED_LRC, this.hooks.settings().transcribeModel);
      else this.empty.add(trackId);
      this.hooks.emitDone(trackId, found);
    }
    this.emit();
    this.pump();
  }

  foundNothing(trackId: string) {
    return this.empty.has(trackId);
  }
}
