/**
 * Mock of downloads (docs/IPC.md "Downloads"): items with their songs, one
 * fake transfer at a time with progress, the storage limit, Go offline and
 * the home-network rule as the core applies them. Test hooks fail the next
 * song, lose a file from disk, or change the connection.
 */
import type { DownloadItem, DownloadKind, DownloadPause, DownloadStatus, Settings } from "../types";

export interface MockDownloadsHooks {
  settings: () => Settings;
  emitStatus: (s: DownloadStatus) => void;
  emitChanged: () => void;
  later: (ms: number, fn: () => void) => void;
  /** An album's, playlist's or Liked Songs' songs now; null for a mix or something unknown. */
  itemTracks: (kind: DownloadKind, id: string) => string[] | null;
  /** Length of a song in ms (sizes are worked out from it). */
  durationMs: (trackId: string) => number;
  /** The server can't be reached (health says only cached music plays). */
  offline: () => boolean;
}

type FileState = "queued" | "done" | "failed" | "missing";

interface MockFile {
  state: FileState;
  bytes: number;
  error?: string;
}

interface MockItem {
  kind: DownloadKind;
  id: string;
  title: string;
  tracks: string[];
}

const GB = 1024 ** 3;
const keyOf = (kind: DownloadKind, id: string) => `${kind}:${kind === "liked" ? "" : id}`;

/** Removing an item that frees more than this asks first. */
export const CONFIRM_REMOVE_BYTES = GB;

export class MockDownloads {
  /** Time per progress step; a song takes `steps` of them. */
  stepMs = 220;
  steps = 6;
  /** Test hooks. */
  failNext = false;
  /** The connection the queue sees ("local" counts as home). */
  connection: "local" | "remote" | "relay" = "local";
  folder = "C:\\Users\\listener\\AppData\\Roaming\\Platter\\downloads";
  freeBytes = 182 * GB;
  /** Bytes counted as used on top of the files (fills storage in tests and demos). */
  usedOffset = 0;

  private items = new Map<string, MockItem>();
  private files = new Map<string, MockFile>();
  private order: string[] = [];
  private current: { trackId: string; received: number; total: number } | null = null;
  private running = false;
  private paused: DownloadPause | null = null;
  /** Bumped by reset, so a transfer still ticking from before stops. */
  private generation = 0;

  constructor(private hooks: MockDownloadsHooks) {}

  /** Back to nothing downloaded (tests). */
  reset() {
    this.generation++;
    this.items.clear();
    this.files.clear();
    this.order = [];
    this.current = null;
    this.running = false;
    this.paused = null;
    this.failNext = false;
    this.connection = "local";
    this.usedOffset = 0;
  }

  private bytesOf(trackId: string): number {
    const q = this.hooks.settings().downloadQuality;
    const kbps = q === "original" ? 1000 : Number(q);
    return Math.round(((kbps * 1000) / 8) * (this.hooks.durationMs(trackId) / 1000));
  }

  private usedBytes(): number {
    let n = this.usedOffset;
    for (const f of this.files.values()) if (f.state === "done") n += f.bytes;
    return n;
  }

  private limitBytes(): number {
    return this.hooks.settings().downloadLimitGb * GB;
  }

  downloadedIds(): string[] {
    return [...this.files.entries()].filter(([, f]) => f.state === "done").map(([id]) => id).sort();
  }

  /** Seeds finished items (demo start states). */
  seedDone(kind: DownloadKind, id: string, title: string) {
    const tracks = this.hooks.itemTracks(kind, id) ?? [];
    this.items.set(keyOf(kind, id), { kind, id: kind === "liked" ? "" : id, title, tracks });
    for (const t of tracks) this.files.set(t, { state: "done", bytes: this.bytesOf(t) });
  }

  status(): DownloadStatus {
    const items: DownloadItem[] = [...this.items.values()].map((it) => {
      let done = 0;
      let failed = 0;
      let missing = 0;
      let error: string | null = null;
      for (const t of it.tracks) {
        const f = this.files.get(t);
        if (f?.state === "done") done++;
        else if (f?.state === "failed") failed++;
        else if (f?.state === "missing") missing++;
        if (!error && f?.error) error = f.error;
      }
      const has = !!this.current && it.tracks.includes(this.current.trackId);
      const fraction = has && this.current ? this.current.received / this.current.total : 0;
      const total = it.tracks.length;
      const state =
        done >= total ? "downloaded" : done + failed + missing >= total ? "error" : has || (done > 0 && !this.paused) ? "downloading" : "queued";
      const own = it.tracks
        .filter((t) => this.files.get(t)?.state === "done" && ![...this.items.values()].some((o) => o !== it && o.tracks.includes(t)))
        .reduce((n, t) => n + (this.files.get(t)?.bytes ?? 0), 0);
      return {
        kind: it.kind,
        id: it.id,
        title: it.title,
        state,
        total,
        done,
        failed,
        missing,
        progress: total === 0 ? 1 : Math.min(1, (done + fraction) / total),
        bytes: own,
        error,
      };
    });
    return {
      items,
      usedBytes: this.usedBytes(),
      limitBytes: this.limitBytes(),
      freeBytes: this.freeBytes,
      folder: this.folder,
      paused: this.paused,
      queued: [...this.files.values()].filter((f) => f.state === "queued").length,
      current: this.current ? { ...this.current } : null,
    };
  }

  private emit() {
    this.hooks.emitStatus(this.status());
  }

  /** Brings the file list in line with the items; returns whether files went. */
  private reconcile(): boolean {
    const wanted = new Set<string>();
    for (const it of this.items.values()) for (const t of it.tracks) wanted.add(t);
    for (const t of wanted)
      if (!this.files.has(t)) {
        this.files.set(t, { state: "queued", bytes: 0 });
        this.order.push(t);
      }
    let removed = false;
    for (const t of [...this.files.keys()])
      if (!wanted.has(t)) {
        this.files.delete(t);
        this.order = this.order.filter((x) => x !== t);
        if (this.current?.trackId === t) this.current = null;
        removed = true;
      }
    return removed;
  }

  set(kind: DownloadKind, id: string, on: boolean, title?: string, trackIds?: string[]): DownloadStatus {
    const key = keyOf(kind, id);
    if (on) {
      const tracks = kind === "mix" ? trackIds : (this.hooks.itemTracks(kind, id) ?? trackIds);
      if (!tracks) throw { kind: kind === "mix" ? "invalid" : "notFound", message: kind === "mix" ? "A mix needs its songs." : "That isn't in your library." };
      const prev = this.items.get(key);
      this.items.set(key, { kind, id: kind === "liked" ? "" : id, title: title || prev?.title || (kind === "liked" ? "Liked Songs" : ""), tracks: [...new Set(tracks)] });
    } else {
      this.items.delete(key);
    }
    const removed = this.reconcile();
    if (removed || on) this.hooks.emitChanged();
    this.emit();
    this.pump();
    return this.status();
  }

  isOn(kind: DownloadKind, id: string): boolean {
    return this.items.has(keyOf(kind, id));
  }

  /** Playlists and Liked Songs follow the library (on every library or like change, as the core does). */
  resync() {
    let changed = false;
    for (const it of this.items.values()) {
      if (it.kind === "mix") continue;
      const now = this.hooks.itemTracks(it.kind, it.id);
      if (now && now.join() !== it.tracks.join()) {
        it.tracks = [...new Set(now)];
        changed = true;
      }
    }
    if (!changed) return;
    if (this.reconcile()) this.hooks.emitChanged();
    this.emit();
    this.pump();
  }

  retry(kind?: DownloadKind, id?: string) {
    const only = kind ? this.items.get(keyOf(kind, id ?? ""))?.tracks : null;
    for (const [t, f] of this.files)
      if ((f.state === "failed" || f.state === "missing") && (!only || only.includes(t))) {
        this.files.set(t, { state: "queued", bytes: 0 });
        this.order = [...this.order.filter((x) => x !== t), t];
      }
    this.emit();
    this.pump();
  }

  removeAll(): number {
    const freed = this.usedBytes();
    this.items.clear();
    this.files.clear();
    this.order = [];
    this.current = null;
    this.hooks.emitChanged();
    this.emit();
    return freed;
  }

  chooseFolder(): string {
    this.folder = "D:\\Music\\Platter downloads";
    this.emit();
    return this.folder;
  }

  /** Test hook: a finished file disappears from disk. */
  loseFile(trackId: string) {
    const f = this.files.get(trackId);
    if (f?.state !== "done") return;
    this.files.set(trackId, { state: "missing", bytes: 0, error: "The file is missing from this computer." });
    this.hooks.emitChanged();
    this.emit();
  }

  /** Settings or health changed: the queue may start or stop. */
  poke() {
    this.emit();
    this.pump();
  }

  private blocked(): DownloadPause | null {
    const s = this.hooks.settings();
    if (s.goOffline || this.hooks.offline()) return "offline";
    if (s.downloadHomeOnly && this.connection !== "local") return "notHome";
    return null;
  }

  private setPaused(p: DownloadPause | null) {
    if (this.paused === p) return;
    this.paused = p;
    this.emit();
  }

  private pump() {
    if (this.running) return;
    const why = this.blocked();
    if (why) return this.setPaused(why);
    const next = this.order.find((t) => this.files.get(t)?.state === "queued");
    if (!next) return this.setPaused(null);
    const total = this.bytesOf(next);
    if (this.usedBytes() + total > this.limitBytes()) return this.setPaused("full");
    this.setPaused(null);
    this.running = true;
    this.current = { trackId: next, received: 0, total };
    this.emit();
    let step = 0;
    const generation = this.generation;
    const tick = () => {
      // Reset meanwhile (the next test): this transfer is gone.
      if (generation !== this.generation) return;
      const cur = this.current;
      // Removed, or the queue has to stop (Go offline, away from home).
      if (!cur || cur.trackId !== next || this.blocked()) {
        if (cur?.trackId === next) this.current = null;
        this.running = false;
        this.emit();
        this.pump();
        return;
      }
      step++;
      if (step >= this.steps) {
        this.current = null;
        this.running = false;
        if (this.failNext) {
          this.failNext = false;
          this.files.set(next, { state: "failed", bytes: 0, error: "The file isn't on the server any more." });
        } else {
          this.files.set(next, { state: "done", bytes: total });
          this.hooks.emitChanged();
        }
        this.emit();
        this.pump();
        return;
      }
      cur.received = Math.round((total * step) / this.steps);
      this.emit();
      this.hooks.later(this.stepMs, tick);
    };
    this.hooks.later(this.stepMs, tick);
  }
}
