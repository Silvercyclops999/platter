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
  /** A song's title. */
  title?: (trackId: string) => string | undefined;
}

const SIZES: Record<TranscribeModel, number> = { small: 190_098_681, medium: 539_225_533, turbo: 574_041_195 };
const MODELS: readonly TranscribeModel[] = ["small", "medium", "turbo"];

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
  /** The graphics card found on this computer (none by default). */
  gpuCard: string | undefined = undefined;
  /** Seconds per song measured here, by model and device. */
  speeds: Partial<Record<`${TranscribeModel}/${"gpu" | "cpu"}`, number>> = {};
  /** Music is playing: reading a song waits for it (test hook). */
  playing = false;
  /** The batch is paused by the user. */
  userPaused = false;
  /** Songs done with a model other than the chosen one. */
  private otherModel = new Map<string, TranscribeModel>();

  private installed = new Set<TranscribeModel>();
  private download: { kind: TranscribeModel; done: number; cancelled: boolean } | null = null;
  private queue: { trackId: string; background: boolean; redo?: boolean; now?: boolean }[] = [];
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
    this.gpuCard = undefined;
    this.speeds = {};
    this.playing = false;
    this.userPaused = false;
    this.otherModel.clear();
  }

  /** Installs a model straight away (tests). */
  install(kind: TranscribeModel) {
    this.installed.add(kind);
    this.emit();
  }

  /** The model a song uses now: the chosen one, else an installed one while it downloads. */
  private runModel(): TranscribeModel | undefined {
    const chosen = this.model();
    if (this.installed.has(chosen)) return chosen;
    if (this.download?.kind !== chosen) return undefined;
    return (["turbo", "medium", "small"] as const).find((k) => this.installed.has(k));
  }

  /** The card in use: one was found and the setting is Auto. */
  private gpu(): string | undefined {
    return this.hooks.settings().transcribeGpu === "off" ? undefined : this.gpuCard;
  }

  /** The model used: the one picked, else Turbo on a graphics card and Small without. */
  model(): TranscribeModel {
    return this.hooks.settings().transcribeModel ?? (this.gpu() ? "turbo" : "small");
  }

  status(): TranscribeStatus {
    const s = this.hooks.settings();
    return {
      enabled: s.transcribeLyrics,
      available: true,
      model: this.model(),
      modelPicked: s.transcribeModel !== null,
      gpu: this.gpu(),
      gpuFound: this.gpuCard,
      models: MODELS.map((kind) => ({
        kind,
        sizeBytes: SIZES[kind],
        installed: this.installed.has(kind),
        downloading: this.download?.kind === kind,
        doneBytes: this.download?.kind === kind ? this.download.done : 0,
        secsPerSong: this.speeds[`${kind}/${this.gpu() ? "gpu" : "cpu"}`],
      })),
      current: this.current ? { ...this.current } : undefined,
      queued: this.queue.length,
      upNext: this.queue[0]?.trackId,
      batch: { ...this.batch },
      secsPerSong: this.batch.total > 1 ? this.secsPerSong : undefined,
      paused: this.busy && this.batch.total > 0 ? "busy" : undefined,
      error: this.error,
      userPaused: this.userPaused,
      redo: this.redoInfo(),
    };
  }

  private redoInfo(): NonNullable<TranscribeStatus["redo"]> {
    const chosen = this.model();
    const out: NonNullable<TranscribeStatus["redo"]> = [];
    for (const m of this.otherModel.values()) {
      if (m === chosen) continue;
      const r = out.find((x) => x.model === m);
      if (r) r.count++;
      else out.push({ model: m, count: 1 });
    }
    return out;
  }

  pause(paused: boolean) {
    this.userPaused = paused;
    this.emit();
    this.pump();
  }

  redo(): number {
    this.usable();
    const chosen = this.model();
    if (!this.installed.has(chosen)) this.refuse("Wait for the model download to finish first.");
    const ids = [...this.otherModel.entries()].filter(([, m]) => m !== chosen).map(([id]) => id);
    const fresh = ids.filter((id) => !this.queue.some((j) => j.trackId === id) && this.current?.trackId !== id);
    this.queue.push(...fresh.map((trackId) => ({ trackId, background: true, redo: true })));
    this.batch.total += fresh.length;
    this.emit();
    this.pump();
    return fresh.length;
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
    const model = this.model();
    if (!this.installed.has(model) && !this.download) this.downloadModel(model);
    const fresh = ids.filter((id) => this.current?.trackId !== id && !this.queue.some((j) => j.trackId === id));
    const jobs = fresh.map((trackId) => ({ trackId, background, now: front }));
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
    const model = this.runModel() ?? this.model();
    let job: { trackId: string; background: boolean } | undefined;
    if (this.current?.stage === "waitingForModel") {
      if (!this.installed.has(model)) return;
      job = { trackId: this.current.trackId, background: this.current.background };
    } else if (this.current) return;
    else {
      // Paused: only songs asked for one at a time.
      const at = this.userPaused ? this.queue.findIndex((j) => j.now) : this.queue.length ? 0 : -1;
      job = at < 0 ? undefined : this.queue.splice(at, 1)[0];
      if (!job && this.userPaused && this.queue.length) {
        this.emit();
        return;
      }
    }
    if (!job) {
      this.batch = { done: 0, total: 0 };
      this.emit();
      return;
    }
    this.current = { trackId: job.trackId, stage: "reading", percent: 0, background: job.background, model, title: this.hooks.title?.(job.trackId) };
    if (this.playing) this.current.waitingForPlayback = true;
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
      if (this.playing) {
        this.hooks.later(this.stepMs, () => step(pct));
        return;
      }
      if (pct > 100) return this.finish(job.trackId, job.background, model);
      this.current = { ...this.current, stage: "transcribing", percent: pct, waitingForPlayback: undefined };
      this.emit();
      this.hooks.later(this.stepMs, () => step(pct + 25));
    };
    this.hooks.later(this.stepMs, () => step(0));
  }

  private finish(trackId: string, background: boolean, model: TranscribeModel) {
    this.running = false;
    this.current = undefined;
    this.batch.done = Math.min(this.batch.total, this.batch.done + 1);
    const skip = background && this.hooks.hasLyrics(trackId);
    if (this.failNextSong) {
      this.failNextSong = false;
      this.error = { kind: "transcribe", message: "Transcription failed for this song.", trackId };
    } else if (!skip) {
      const found = !this.silent.has(trackId);
      if (found) this.hooks.save(trackId, CANNED_LRC, model);
      else this.empty.add(trackId);
      if (model !== this.model()) this.otherModel.set(trackId, model);
      else this.otherModel.delete(trackId);
      this.hooks.emitDone(trackId, found);
    }
    this.emit();
    this.pump();
  }

  foundNothing(trackId: string) {
    return this.empty.has(trackId);
  }
}
