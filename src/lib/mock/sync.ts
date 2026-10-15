/**
 * Mock sync file, pins, library folders and key copy. Paths and device
 * names are invented.
 *
 * Test hooks: `setFileState(state)` ("folderMissing", "unreadable", ...),
 * `otherCode` (the code the pretend other device shows), `noDevice`
 * (nobody is showing a code), `keysArrive(kinds)` (the pretend other device
 * sends keys to this one), `expireCode()`.
 */
import type {
  CloudFolder,
  KeyKind,
  KeyReceiveState,
  KeySendOutcome,
  LibraryFolder,
  LibraryFolders,
  PinRecord,
  Settings,
  SyncFileState,
  SyncFileStatus,
  SyncStoreChanged,
} from "../types";

export interface SyncHooks {
  settings: () => Settings;
  setSettings: (patch: Partial<Settings>) => void;
  emitStatus: (s: SyncFileStatus) => void;
  emitChanged: (c: SyncStoreChanged) => void;
  emitKeys: (s: KeyReceiveState) => void;
}

export const MOCK_CLOUD_FOLDERS: CloudFolder[] = [
  { provider: "OneDrive", path: "C:\\Users\\listener\\OneDrive" },
  { provider: "Dropbox", path: "C:\\Users\\listener\\Dropbox" },
];

const MAX_PINS = 12;
const TTL_MS = 5 * 60 * 1000;
const IDLE: KeyReceiveState = { phase: "idle", code: null, qr: null, expiresAt: null, from: null, received: [] };

/** A stand-in QR pattern (finder squares and a seeded fill), 25 modules wide. */
export function mockQr(code: string): { size: number; rows: string[] } {
  const size = 25;
  let seed = [...code].reduce((a, c) => a * 31 + c.charCodeAt(0), 7) >>> 0;
  const rnd = () => ((seed = (seed * 1103515245 + 12345) >>> 0) >> 16) & 1;
  const finder = (r: number, c: number) => {
    for (const [fr, fc] of [
      [0, 0],
      [0, size - 7],
      [size - 7, 0],
    ]) {
      const y = r - fr;
      const x = c - fc;
      if (y >= 0 && y < 7 && x >= 0 && x < 7) return y === 0 || y === 6 || x === 0 || x === 6 || (y >= 2 && y <= 4 && x >= 2 && x <= 4) ? 1 : 0;
      if (y >= -1 && y <= 7 && x >= -1 && x <= 7) return 0;
    }
    return null;
  };
  const rows: string[] = [];
  for (let r = 0; r < size; r++) {
    let row = "";
    for (let c = 0; c < size; c++) row += String(finder(r, c) ?? rnd());
    rows.push(row);
  }
  return { size, rows };
}

export class MockSync {
  fileState: SyncFileState = "ok";
  otherCode = "246810";
  noDevice = false;
  available: KeyKind[] = ["listenbrainz", "lastfm", "imgur", "catbox"];
  private pins: PinRecord[] = [];
  private folders: LibraryFolder[] = [];
  private items: Record<string, string> = {};
  private receive: KeyReceiveState = { ...IDLE };
  private lastSynced: number | null = null;
  private seq = 0;

  constructor(private hooks: SyncHooks) {}

  reset() {
    this.fileState = "ok";
    this.otherCode = "246810";
    this.noDevice = false;
    this.pins = [];
    this.folders = [];
    this.items = {};
    this.receive = { ...IDLE };
    this.lastSynced = null;
    this.available = ["listenbrainz", "lastfm", "imgur", "catbox"];
    this.hooks.setSettings({ syncFolder: "", syncEnabled: false });
  }

  // ---------------------------------------------------------------- the file

  status(): SyncFileStatus {
    const s = this.hooks.settings();
    const on = s.syncEnabled && !!s.syncFolder;
    if (!on) return { state: "off", folder: s.syncFolder || null, file: null, lastSyncedAt: null, devices: [], mergedCopies: 0, message: null };
    const file = `${s.syncFolder}\\Platter\\sync-0123456789abcdef.json`;
    const message: Partial<Record<SyncFileState, string>> = {
      folderMissing: "The sync folder isn't there. Changes are kept on this computer until it's back.",
      unreadable: "The sync file can't be read. Changes are kept on this computer. Start a new file to sync again.",
      newer: "A newer Platter wrote the sync file. Update Platter to keep syncing.",
      signedOut: "Sign in to Plex to sync.",
    };
    if (this.fileState === "ok") this.lastSynced = Date.now();
    return {
      state: this.fileState,
      folder: s.syncFolder,
      file,
      lastSyncedAt: this.lastSynced,
      devices:
        this.fileState === "ok"
          ? [
              { name: "This computer", lastSeen: Date.now(), thisDevice: true },
              { name: "Studio Mac", lastSeen: Date.now() - 2 * 3600_000, thisDevice: false },
            ]
          : [],
      mergedCopies: 0,
      message: message[this.fileState] ?? null,
    };
  }

  /** Test hook: the file or folder went into a state. */
  setFileState(state: SyncFileState) {
    this.fileState = state;
    this.hooks.emitStatus(this.status());
  }

  useFolder(path: string): SyncFileStatus {
    if (!path.trim()) throw { kind: "notFound", message: "That folder isn't there." };
    this.hooks.setSettings({ syncFolder: path, syncEnabled: true });
    const st = this.status();
    this.hooks.emitStatus(st);
    return st;
  }

  replace(): SyncFileStatus {
    this.fileState = "ok";
    const st = this.status();
    this.hooks.emitStatus(st);
    return st;
  }

  // ---------------------------------------------------------------- pins

  pinsList(): PinRecord[] {
    return [...this.pins].sort((a, b) => (b.pinnedAt ?? 0) - (a.pinnedAt ?? 0)).slice(0, MAX_PINS);
  }

  pinsToggle(pin: PinRecord): PinRecord[] {
    if (!["album", "artist", "playlist"].includes(pin.kind)) throw { kind: "invalid", message: "Only albums, artists and playlists can be pinned." };
    const i = this.pins.findIndex((p) => p.kind === pin.kind && p.id === pin.id);
    if (i >= 0) this.pins.splice(i, 1);
    else this.pins = [{ ...pin, pinnedAt: Date.now() + this.seq++ }, ...this.pins].slice(0, MAX_PINS);
    this.changed("pins");
    return this.pinsList();
  }

  pinsImport(pins: PinRecord[]): PinRecord[] {
    if (this.pins.length === 0) this.pins = pins.slice(0, MAX_PINS).map((p, i) => ({ ...p, pinnedAt: Date.now() - i }));
    return this.pinsList();
  }

  // ---------------------------------------------------------------- folders

  tree(): LibraryFolders {
    const known = new Set(this.folders.map((f) => f.id));
    const items = Object.fromEntries(Object.entries(this.items).filter(([, f]) => known.has(f)));
    return { folders: [...this.folders].sort((a, b) => b.createdAt - a.createdAt), items };
  }

  private folder(id: string): LibraryFolder {
    const f = this.folders.find((x) => x.id === id);
    if (!f) throw { kind: "notFound", message: "That folder isn't there any more." };
    return f;
  }

  private ancestors(id: string): string[] {
    const out: string[] = [];
    let at = this.folders.find((f) => f.id === id)?.parent ?? null;
    while (at && !out.includes(at)) {
      out.push(at);
      at = this.folders.find((f) => f.id === at)?.parent ?? null;
    }
    return out;
  }

  private cleanName(name: string): string {
    const n = name.trim();
    if (!n) throw { kind: "invalid", message: "Folders need a name." };
    if ([...n].length > 100) throw { kind: "invalid", message: "Folder names can be up to 100 characters." };
    return n;
  }

  private changed(...namespaces: string[]) {
    this.hooks.emitChanged({ namespaces, remote: false });
  }

  create(name: string | null | undefined, parent: string | null | undefined): LibraryFolder {
    if (parent) {
      this.folder(parent);
      if (this.ancestors(parent).length + 2 > 8) throw { kind: "invalid", message: "Folders can only go 8 deep." };
    }
    const f: LibraryFolder = { id: `folder${++this.seq}`, name: this.cleanName(name ?? "New Folder"), parent: parent ?? null, createdAt: Date.now() + this.seq };
    this.folders.push(f);
    this.changed("folders");
    return f;
  }

  rename(id: string, name: string) {
    this.folder(id).name = this.cleanName(name);
    this.changed("folders");
  }

  remove(id: string) {
    const gone = this.folder(id);
    for (const f of this.folders) if (f.parent === id) f.parent = gone.parent;
    for (const [p, f] of Object.entries(this.items)) {
      if (f !== id) continue;
      if (gone.parent) this.items[p] = gone.parent;
      else delete this.items[p];
    }
    this.folders = this.folders.filter((f) => f.id !== id);
    this.changed("folders", "folderItems");
  }

  move(id: string, parent: string | null) {
    const f = this.folder(id);
    if (parent) {
      this.folder(parent);
      if (parent === id || this.ancestors(parent).includes(id)) throw { kind: "invalid", message: "A folder can't go inside itself." };
    }
    f.parent = parent;
    this.changed("folders");
  }

  place(playlistId: string, folderId: string | null) {
    if (!playlistId || playlistId === "liked") throw { kind: "invalid", message: "Only playlists go in folders." };
    if (folderId) {
      this.folder(folderId);
      this.items[playlistId] = folderId;
    } else delete this.items[playlistId];
    this.changed("folderItems");
  }

  // ---------------------------------------------------------------- keys

  receiveState(): KeyReceiveState {
    const r = this.receive;
    if ((r.phase === "waiting" || r.phase === "receiving") && r.expiresAt !== null && Date.now() >= r.expiresAt) {
      this.receive = { ...IDLE, phase: "expired" };
    }
    return this.receive;
  }

  receiveStart(): KeyReceiveState {
    const code = String(100000 + Math.floor(Math.random() * 900000));
    this.receive = { phase: "waiting", code, qr: mockQr(code), expiresAt: Date.now() + TTL_MS, from: null, received: [] };
    this.hooks.emitKeys(this.receive);
    return this.receive;
  }

  receiveCancel() {
    this.receive = { ...IDLE };
    this.hooks.emitKeys(this.receive);
  }

  /** Test/demo hook: the other device sends keys. */
  keysArrive(kinds: KeyKind[], from = "Studio Mac") {
    if (this.receiveState().phase !== "waiting") return;
    for (const k of kinds) if (!this.available.includes(k)) this.available.push(k);
    this.receive = { ...IDLE, phase: "received", from, received: kinds };
    this.hooks.emitKeys(this.receive);
  }

  /** Test hook: the code ran out. */
  expireCode() {
    if (this.receive.phase === "waiting") {
      this.receive = { ...IDLE, phase: "expired" };
      this.hooks.emitKeys(this.receive);
    }
  }

  send(code: string, kinds: KeyKind[]): KeySendOutcome {
    const c = code.replace(/[\s-]/g, "");
    if (!/^\d{6}$/.test(c)) throw { kind: "invalid", message: "The code is 6 digits." };
    const sent = kinds.filter((k) => this.available.includes(k));
    if (sent.length === 0) throw { kind: "invalid", message: "Choose at least one key this computer has." };
    if (this.noDevice)
      throw {
        kind: "notFound",
        message: "No device is showing a code. On the other computer, open Settings and choose Copy keys from another device.",
      };
    if (c !== this.otherCode) throw { kind: "invalid", message: "That code doesn't match. Check the code on the other computer." };
    return { device: "Studio Mac", sent };
  }
}
