import type {
  Album,
  AlbumUpgrade,
  AppError,
  OfferSong,
  Settings,
  ShareMap,
  SwapResult,
  Track,
  UpgradeBulkResult,
  UpgradeOffer,
  UpgradeRecord,
  UpgradeUnder,
} from "../types";

/**
 * Lidarr upgrades in the browser mock. `?upgrades=` picks a start:
 * `ready` (staging folder and share mapping set, the first album's better
 * copy imported and offered; only "the new copy isn't better" is kept, so
 * it asks once), `mixed` (the same with an archive-tagged song and a stray
 * leak in the folder, so it asks per song), `locked` (the swap fails on a
 * file in use) or `noMap` (imported, but no share mapping yet).
 */
export type UpgradeDemo = "ready" | "mixed" | "locked" | "noMap";
export const UPGRADE_DEMOS: UpgradeDemo[] = ["ready", "mixed", "locked", "noMap"];
export const isUpgradeDemo = (v: string | null): v is UpgradeDemo => !!v && (UPGRADE_DEMOS as string[]).includes(v);

export const MOCK_STAGING = "/downloads/upgrades";
export const MOCK_BACKUP = "D:\\Platter backups";
const MOCK_SHARE: ShareMap = { music: [{ server: "/data/media/music", local: "M:\\" }], staging: "U:\\", backup: MOCK_BACKUP };

const invalid = (message: string): AppError => ({ kind: "invalid", message });

interface Deps {
  album: (id: string) => Album | undefined;
  albums: () => Album[];
  tracks: (albumId: string) => Track[];
  settings: () => Settings;
  lidarrReady: () => boolean;
  now: () => number;
}

/** Same words as the core's quality labels. */
export function qualityLabel(t: Pick<Track, "codec" | "bitrate" | "bitDepth">): string {
  const c = (t.codec ?? "").toUpperCase();
  if (["FLAC", "ALAC", "WAV", "AIFF"].includes(c)) return t.bitDepth && t.bitDepth !== 16 ? `${c} ${t.bitDepth}-bit` : c;
  return t.bitrate ? `${c} ${t.bitrate}` : c;
}

const rank = (t: Pick<Track, "codec" | "bitrate" | "bitDepth">) =>
  ["flac", "alac", "wav", "aiff"].includes((t.codec ?? "").toLowerCase()) ? 1000 + (t.bitDepth ?? 16) * 10 : (t.bitrate ?? 1);

export class MockUpgrades {
  records: UpgradeRecord[] = [];
  share: ShareMap = { music: [], staging: "", backup: "" };
  private demo: UpgradeDemo | undefined;

  constructor(
    private deps: Deps,
    demo?: UpgradeDemo,
  ) {
    this.demo = demo;
  }

  /** Test hook / start: set up the demo (call after settings are in place). */
  reset(demo?: UpgradeDemo) {
    this.demo = demo;
    this.records = [];
    this.share = demo && demo !== "noMap" ? structuredClone(MOCK_SHARE) : { music: [], staging: "", backup: "" };
    if (demo) {
      const a = this.deps.albums()[0];
      if (a) {
        const r = this.newRecord(a);
        r.state = "imported";
        this.records = [r];
      }
    }
  }

  private current(albumId: string): { rank: number; label: string } | null {
    const ts = this.deps.tracks(albumId);
    if (!ts.length) return null;
    const worst = [...ts].sort((x, y) => rank(x) - rank(y))[0];
    return { rank: rank(worst), label: qualityLabel(worst) };
  }

  private on() {
    return !!this.deps.settings().lidarrStagingFolder.trim() && this.deps.lidarrReady();
  }

  forAlbum(albumId: string): AlbumUpgrade {
    const album = this.deps.album(albumId);
    const now = this.current(albumId);
    const record = this.records.find((r) => r.albumId === albumId) ?? null;
    const released = !!album && album.format !== null;
    return { available: this.on() && released && !!now && now.rank < 1240, record, quality: this.on() ? (now?.label ?? null) : null };
  }

  private newRecord(a: Album): UpgradeRecord {
    const t = this.deps.now();
    const n = this.records.length + 1;
    return {
      id: `upg-${n}-${a.id}`,
      releaseGroupId: `9b1c0c55-0000-4000-8000-${String(n).padStart(12, "0")}`,
      title: a.title,
      artist: a.artistTitle,
      artistId: a.artistId,
      state: "downloading",
      destination: `${this.deps.settings().lidarrStagingFolder || MOCK_STAGING}/${a.artistTitle}`,
      lidarrAlbumId: 500 + n,
      addedArtist: true,
      alreadyInLidarr: false,
      createdAt: t,
      updatedAt: t,
      message: null,
      notes: [`Asked Lidarr for ${this.target(a.id)} or better with its Lossless profile.`],
      auto: false,
      albumId: a.id,
      fromQuality: this.current(a.id)?.label ?? "",
      swaps: [],
      failure: null,
    };
  }

  private target(albumId: string) {
    const now = this.current(albumId);
    return now && now.rank >= 1160 ? "FLAC 24-bit" : "FLAC";
  }

  request(albumId: string): UpgradeRecord {
    if (!this.deps.settings().lidarrStagingFolder.trim()) throw invalid("Set the Lidarr staging folder for upgrades in Settings first.");
    if (!this.deps.lidarrReady()) throw invalid("Platter won't send Lidarr anything until its setup check passes. Check it in Settings.");
    const a = this.deps.album(albumId);
    if (!a) throw invalid("That album isn't in the library any more.");
    if (a.format === null) throw invalid("This album counts as unreleased, so Platter won't look for another copy of it.");
    const had = this.records.find((r) => r.albumId === albumId && r.state !== "failed" && !r.swaps.length);
    if (had) return had;
    const r = this.newRecord(a);
    this.records = [r, ...this.records];
    return r;
  }

  bulk(under: UpgradeUnder): UpgradeBulkResult {
    const limit = under === "lossy" ? 1000 : 1160;
    const have = new Set(this.records.map((r) => r.albumId));
    const wanted = this.deps.albums().filter((a) => !have.has(a.id) && a.format !== null && (this.current(a.id)?.rank ?? 9999) < limit);
    const requested = wanted.slice(0, 25).map((a) => this.request(a.id));
    return { requested, skipped: [], more: Math.max(0, wanted.length - 25) };
  }

  /** Refresh: downloads in the mock finish when looked at again. */
  list(refresh: boolean): UpgradeRecord[] {
    if (refresh) {
      this.records = this.records.map((r) => (r.state === "downloading" || r.state === "searching" ? { ...r, state: "imported", updatedAt: this.deps.now() } : r));
    }
    return [...this.records];
  }

  private record(id: string) {
    const r = this.records.find((x) => x.id === id);
    if (!r) throw invalid("That upgrade isn't on this computer any more.");
    return r;
  }

  private save(r: UpgradeRecord) {
    this.records = this.records.map((x) => (x.id === r.id ? r : x));
  }

  offer(id: string): UpgradeOffer {
    const r = this.record(id);
    const last = [...r.swaps].reverse().find((s) => !s.undone) ?? null;
    const base: UpgradeOffer = {
      upgradeId: r.id,
      albumId: r.albumId,
      title: r.title,
      artist: r.artist,
      status: "waiting",
      message: null,
      summary: null,
      from: r.fromQuality,
      to: "",
      upgrade: [],
      kept: [],
      askEach: false,
      askReason: null,
      ask: this.deps.settings().upgradesAsk,
      backupDir: this.share.backup || "Platter's folder on this computer",
      lastSwap: last,
      failure: r.failure,
      failureMessage: r.failure ? failureMessage(r.failure) : null,
    };
    if (last) return { ...base, status: "swapped", summary: last.summary };
    if (r.state !== "imported") return { ...base, message: r.state === "failed" ? (r.message ?? "Lidarr couldn't get a better copy.") : "Lidarr is still getting the better copy." };
    if (!this.share.music.length) {
      return { ...base, status: "needsMapping", message: "Set where this computer reaches your music share (Settings > Integrations > Lidarr) to see what would change." };
    }
    const tracks = this.deps.tracks(r.albumId);
    const to = this.target(r.albumId);
    const song = (t: Track): OfferSong => ({ trackId: t.id, title: t.title, disc: t.discNumber, track: t.trackNumber, from: qualityLabel(t), to, reason: null, keep: null, by: "recordingId" });
    const upgrade: OfferSong[] = [];
    const kept: OfferSong[] = [];
    tracks.forEach((t, i) => {
      if (this.demo === "mixed" && i === 1) kept.push({ ...song(t), to: null, by: null, reason: "Has archive tags", keep: "archiveTags" });
      else if (i === tracks.length - 1 && tracks.length > 2) kept.push({ ...song(t), to: null, by: null, reason: "The new copy isn't better", keep: "notBetter" });
      else upgrade.push(song(t));
    });
    if (this.demo === "mixed") {
      kept.push({ trackId: "", title: "Bonus Track (Leak).mp3", disc: 0, track: 0, from: "MP3 192", to: null, reason: "Not one of this album's songs in Plex", keep: "notInAlbum", by: null });
    }
    const askEach = kept.some((k) => k.keep !== "notBetter");
    return {
      ...base,
      status: upgrade.length ? "ready" : "nothing",
      upgrade,
      kept,
      to,
      summary: `Upgrade ${upgrade.length} of ${tracks.length} ${tracks.length === 1 ? "song" : "songs"} (${r.fromQuality} to ${to})`,
      askEach,
      askReason: askEach
        ? "Some files in this album's folders are kept as they are (unreleased, archive tags, an extra disc or no proof of a match), so Platter asks about each song."
        : null,
    };
  }

  swap(id: string, trackIds: string[] | null): SwapResult {
    const o = this.offer(id);
    if (o.status !== "ready") throw invalid(o.message ?? "There's nothing to swap.");
    if (trackIds === null && o.askEach) throw invalid(o.askReason ?? "Pick each song to upgrade.");
    const chosen = o.upgrade.filter((s) => trackIds === null || trackIds.includes(s.trackId));
    if (!chosen.length) throw invalid("Pick at least one song to upgrade.");
    const r = this.record(id);
    if (this.demo === "locked") {
      const failure = { kind: "fileLocked" as const, name: `${String(chosen[0].track).padStart(2, "0")} ${chosen[0].title}.mp3` };
      this.save({ ...r, failure });
      throw invalid(failureMessage(failure));
    }
    const at = this.deps.now();
    const backupDir = `${this.share.backup || "Platter backups"}\\${r.artist} - ${r.title} (${r.id.slice(-8)})`;
    const summary = `Upgraded ${chosen.length} ${chosen.length === 1 ? "song" : "songs"} (${o.from} to ${o.to})`;
    const run = {
      id: `swap-${at}-${r.swaps.length}`,
      at,
      backupDir,
      files: chosen.map((s) => ({ trackId: s.trackId, plexPath: "", original: "", placed: "", backup: "", oldHash: "", newHash: "" })),
      undone: false,
      summary,
    };
    const next = { ...r, swaps: [...r.swaps, run], failure: null };
    this.save(next);
    return { record: next, swapped: chosen.length, backupDir, summary };
  }

  undo(id: string): UpgradeRecord {
    const r = this.record(id);
    const last = [...r.swaps].reverse().find((s) => !s.undone);
    if (!last) throw invalid("There's no swap to undo.");
    const next = { ...r, swaps: r.swaps.map((s) => (s.id === last.id ? { ...s, undone: true } : s)), failure: null };
    this.save(next);
    return next;
  }

  clear(id: string): UpgradeRecord[] {
    const r = this.records.find((x) => x.id === id);
    if (r && r.swaps.some((s) => !s.undone)) throw invalid("Undo the swap first, or keep it in the list so it can be undone.");
    this.records = this.records.filter((x) => x.id !== id);
    return this.list(false);
  }

  setShare(map: ShareMap): ShareMap {
    const abs = (p: string) => !p || /^([A-Za-z]:|\/|\\\\)/.test(p);
    const music = map.music.map((m) => ({ server: m.server.trim(), local: m.local.trim() })).filter((m) => m.server || m.local);
    if (music.some((m) => !m.server || !m.local)) throw invalid("Give both the folder as Plex sees it and where this computer reaches it.");
    if (![...music.flatMap((m) => [m.server, m.local]), map.staging.trim(), map.backup.trim()].every(abs)) throw invalid("Write each folder as a full folder path.");
    this.share = { music, staging: map.staging.trim(), backup: map.backup.trim() };
    return structuredClone(this.share);
  }
}

/** Same words as the core's failure messages. */
export function failureMessage(f: UpgradeRecord["failure"] & object): string {
  switch (f.kind) {
    case "noMapping":
      return "Platter doesn't know where this computer reaches your music share. Set it in Settings > Integrations > Lidarr, then try again. Nothing was changed.";
    case "shareUnreachable":
      return `Platter can't reach ${f.path}. Check the share is connected, then try again. Nothing was changed.`;
    case "fileLocked":
      return `${f.name} is in use by another program. Close it, then try again. Nothing was changed.`;
    case "tagCopyFailed":
      return `${f.why} The swap stopped at ${f.name} and every file is back as it was.`;
    case "backupNotWritable":
      return `Platter can't write to the backup folder ${f.path}, so it changed nothing. Pick another backup folder in Settings.`;
    case "rescanFailed":
      return "Plex didn't take the request to rescan the folder, so Platter put the old files back. Try again when Plex is reachable.";
    case "plexUnreachable":
      return "Platter can't reach Plex to rescan the folder afterwards, so it changed nothing. Try again when Plex is reachable.";
    default:
      return "The swap stopped and every file is back as it was.";
  }
}
