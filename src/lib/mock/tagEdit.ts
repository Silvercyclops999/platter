/**
 * Mock of the tag editor (docs/IPC.md "Tag editor") over the fixture
 * library: files with tags, saving with the changed-file check, undo, the
 * tag check, the one-click fixes and the Plex library settings check.
 * Test hooks: `changeOnDisk` (another program retags a file), `failNext`
 * (the next save fails the way a share would), `lookupDown`, `folderGone`.
 */
import type {
  IsrcRelease,
  IsrcReleases,
  MusicFolderCheck,
  PlexLibraryCheck,
  Settings,
  TagCheckReport,
  TagCheckScope,
  TagEditFile,
  TagEditorData,
  TagField,
  TagFinding,
  TagFixChange,
  TagFixKind,
  TagFixPreview,
  TagRefusal,
  TagSaveRequest,
  TagSaveResult,
  TagUndoInfo,
  TagValues,
  Track,
} from "../types";
import type { Fixtures } from "./fixtures";

const SERVER_ROOT = "/data/music";
export const NEEDS_FOLDER = "Set up the music folder in Settings, Library to edit tags.";
export const LOOKUP_DOWN = "MusicBrainz and Deezer couldn't be reached. Nothing was changed; try again later.";

interface MockFile {
  values: TagValues;
  stamp: number;
}

interface Entry {
  info: TagUndoInfo;
  files: { trackId: string; before: TagValues; removed: TagField[]; stamp: string }[];
}

const files = (n: number) => (n === 1 ? "1 file" : `${n} files`);

export class MockTagEdit {
  private disk = new Map<string, MockFile>();
  private log: Entry[] = [];
  private ignored = new Set<string>();
  private seq = 1;
  /** The next save fails on its first file this way. */
  failNext: TagRefusal | null = null;
  /** ISRC lookups can't reach the catalogues. */
  lookupDown = false;
  /** The music folder's share is disconnected. */
  folderGone = false;
  /** Tags a file starts with beyond its track's ("" leaves the tag out): the library doctor's odd files. */
  seed: ((trackId: string) => TagValues | undefined) | null = null;

  constructor(
    private fx: Fixtures,
    private settings: () => Settings,
    private setSettings: (patch: Partial<Settings>) => void,
    private changed: () => void,
  ) {}

  private serverPath(t: Track): string {
    const n = String(t.trackNumber).padStart(2, "0");
    return `${SERVER_ROOT}/${t.artistTitle}/${t.albumTitle}/${n} - ${t.title}.flac`;
  }

  private file(t: Track): MockFile {
    let f = this.disk.get(t.id);
    if (!f) {
      const album = this.fx.albums.find((a) => a.id === t.albumId);
      const values: TagValues = {
        albumArtist: t.artistTitle,
        album: t.albumTitle,
        title: t.title,
        artist: t.trackArtist ?? t.artistTitle,
        trackNumber: String(t.trackNumber),
        archiveVersion: album?.format ? "released" : undefined,
      };
      if (album?.year) values.year = String(album.year);
      Object.assign(values, this.seed?.(t.id));
      for (const k of Object.keys(values) as TagField[]) if (values[k] === undefined || values[k] === "") delete values[k];
      f = { values, stamp: 1 };
      this.disk.set(t.id, f);
    }
    return f;
  }

  /** Every file back to its first tags (test hook). */
  resetDisk() {
    this.disk.clear();
    this.log = [];
  }

  /** Another program retags a file (test hook). */
  changeOnDisk(trackId: string, field: TagField = "title", value = "Changed elsewhere") {
    const t = this.fx.tracks.find((x) => x.id === trackId);
    if (!t) return;
    const f = this.file(t);
    f.values = { ...f.values, [field]: value };
    f.stamp += 1;
  }

  private hasFolder(): boolean {
    return !!this.settings().musicFolder && !!this.settings().musicFolderServer;
  }

  private tracksFor(albumId: string | null, trackIds: string[]): Track[] {
    const out = albumId ? this.fx.tracksByAlbum.get(albumId) ?? [] : this.fx.tracks.filter((t) => trackIds.includes(t.id));
    if (!out.length) throw { kind: "notFound", message: "Those songs aren't in the library any more." };
    return out;
  }

  open(albumId: string | null, trackIds: string[]): TagEditorData {
    const tracks = this.tracksFor(albumId, trackIds);
    const folder = this.hasFolder();
    const editFiles: TagEditFile[] = tracks.map((t) => {
      const f = this.file(t);
      const server = this.serverPath(t);
      const readable = folder && !this.folderGone;
      return {
        trackId: t.id,
        albumId: t.albumId,
        serverPath: server,
        localPath: folder ? server.replace(SERVER_ROOT, this.settings().musicFolder) : null,
        fileName: server.split("/").pop() ?? t.title,
        format: "flac",
        id3Version: null,
        values: { ...f.values },
        keys: [],
        stamp: readable ? `${t.id}:${f.stamp}` : null,
        error: folder && this.folderGone ? `${t.title}.flac isn't in the music folder any more. Check that the share is connected.` : null,
        durationMs: t.durationMs,
      };
    });
    const readable = editFiles.filter((f) => f.stamp).length;
    return {
      title: albumId ? tracks[0].albumTitle : tracks.length === 1 ? tracks[0].title : files(tracks.length),
      files: editFiles,
      scheme: "archive",
      statusField: "archiveVersion",
      statusChoices: ["released", "alternate", "demo", "session", "leak", "snippet", "remix", "live", "build"],
      customKey: null,
      writable: readable > 0,
      reason: !folder ? NEEDS_FOLDER : readable === 0 ? editFiles[0].error : null,
      needsMusicFolder: !folder,
    };
  }

  save(req: TagSaveRequest): TagSaveResult {
    if (!this.hasFolder()) throw { kind: "invalid", message: NEEDS_FOLDER };
    const empty: TagSaveResult = { saved: 0, unchanged: 0, problems: [], message: "", undoId: null, rescanned: [], rescanError: null };
    const plan: { t: Track; f: MockFile; edits: TagValues }[] = [];
    for (const sf of req.files) {
      const t = this.fx.tracks.find((x) => x.id === sf.trackId);
      if (!t) continue;
      const f = this.file(t);
      const name = this.serverPath(t).split("/").pop()!;
      const fail = this.failNext;
      if (fail) {
        this.failNext = null;
        const message =
          fail === "busy"
            ? "Another program is changing this folder. Try again shortly."
            : fail === "readOnly"
              ? `Platter can't write to the folder ${name} is in. The share may be read-only for this account.`
              : fail === "locked"
                ? `${name} is open in another program. Close it and try again.`
                : `${name} changed since the editor opened. Open it again to see the new tags.`;
        return { ...empty, problems: [{ trackId: t.id, fileName: name, why: fail, message }], message: fail === "busy" ? message : `Nothing was saved. ${message}` };
      }
      if (`${t.id}:${f.stamp}` !== sf.stamp) {
        const message = `${name} changed since the editor opened. Open it again to see the new tags.`;
        return { ...empty, problems: [{ trackId: t.id, fileName: name, why: "changed", message }], message: `Nothing was saved. ${message}` };
      }
      plan.push({ t, f, edits: sf.edits });
    }
    const entry: Entry = { info: { id: `te${this.seq++}`, at: Date.now(), label: req.label || "Edited tags", files: 0 }, files: [] };
    const folders = new Set<string>();
    let saved = 0;
    for (const { t, f, edits } of plan) {
      const before: TagValues = {};
      const removed: TagField[] = [];
      let changed = false;
      for (const [k, v] of Object.entries(edits) as [TagField, string][]) {
        if ((f.values[k] ?? "") === v) continue;
        changed = true;
        if (f.values[k] === undefined) removed.push(k);
        else before[k] = f.values[k];
        if (v === "") delete f.values[k];
        else f.values[k] = v;
      }
      if (!changed) continue;
      f.stamp += 1;
      saved += 1;
      folders.add(this.serverPath(t).split("/").slice(0, -1).join("/"));
      entry.files.push({ trackId: t.id, before, removed, stamp: `${t.id}:${f.stamp}` });
    }
    if (saved > 0) {
      entry.info.files = saved;
      this.log.unshift(entry);
      this.log = this.log.slice(0, 20);
      this.changed();
    }
    const rescan = this.settings().rescanAfterTagEdit !== false;
    return {
      ...empty,
      saved,
      unchanged: plan.length - saved,
      message: saved ? `Saved ${files(saved)}` : "Nothing changed.",
      undoId: saved ? entry.info.id : null,
      rescanned: rescan ? [...folders] : [],
    };
  }

  undo(id: string): TagSaveResult {
    const i = this.log.findIndex((e) => e.info.id === id);
    if (i < 0) throw { kind: "notFound", message: "That edit can't be undone any more." };
    const [entry] = this.log.splice(i, 1);
    const result: TagSaveResult = { saved: 0, unchanged: 0, problems: [], message: "", undoId: null, rescanned: [], rescanError: null };
    for (const u of entry.files) {
      const t = this.fx.tracks.find((x) => x.id === u.trackId)!;
      const f = this.file(t);
      const name = this.serverPath(t).split("/").pop()!;
      if (`${t.id}:${f.stamp}` !== u.stamp) {
        result.problems.push({ trackId: t.id, fileName: name, why: "changed", message: `${name} changed after the edit, so it was kept as it is.` });
        continue;
      }
      Object.assign(f.values, u.before);
      for (const k of u.removed) delete f.values[k];
      f.stamp += 1;
      result.saved += 1;
    }
    result.message =
      result.problems.length === 0
        ? `Undid the edit on ${files(result.saved)}`
        : result.saved === 0
          ? `Nothing was undone: ${files(result.problems.length)} changed since.`
          : `Undid the edit on ${files(result.saved)}. ${files(result.problems.length)} changed since and kept as they are.`;
    if (result.saved) this.changed();
    return result;
  }

  history(): TagUndoInfo[] {
    return this.log.map((e) => ({ ...e.info }));
  }

  // ---------------------------------------------------------------- the check

  check(scope: TagCheckScope): TagCheckReport {
    const albums =
      scope.kind === "album"
        ? this.fx.albums.filter((a) => a.id === scope.id)
        : scope.kind === "artist"
          ? this.fx.albums.filter((a) => a.artistId === scope.id)
          : this.fx.albums.slice(0, 40);
    const folder = this.hasFolder();
    const findings: TagFinding[] = [];
    let total = 0;
    let ignored = 0;
    for (const a of albums) {
      const tracks = this.fx.tracksByAlbum.get(a.id) ?? [];
      total += tracks.length;
      const vals = tracks.map((t) => ({ t, v: this.file(t).values }));
      const add = (f: Omit<TagFinding, "albumId" | "albumTitle" | "fix" | "id">, field: TagField | null) => {
        const id = `${f.rule}:${a.id}`;
        if (this.ignored.has(id)) {
          ignored += 1;
          return;
        }
        findings.push({ ...f, id, albumId: a.id, albumTitle: a.title, fix: folder ? { albumId: a.id, trackIds: f.trackIds, field } : null });
      };
      const noAa = vals.filter((x) => !x.v.albumArtist).map((x) => x.t.id);
      if (noAa.length)
        add(
          {
            rule: "missingAlbumArtist",
            severity: "error",
            trackIds: noAa,
            what: `${noAa.length === 1 ? "1 song" : `${noAa.length} songs`} without an album artist`,
            why: "Plex decides the artist page by the album artist. Without one, it treats the album as Various Artists.",
          },
          "albumArtist",
        );
      const years = new Set(vals.map((x) => x.v.year).filter(Boolean));
      if (years.size > 1)
        add(
          {
            rule: "albumTagsDiffer",
            severity: "warning",
            trackIds: tracks.map((t) => t.id),
            what: `The year differs between songs: ${[...years].join(", ")}`,
            why: "Plex keeps one value for the whole album, so the album can split in two or show the wrong one.",
          },
          "year",
        );
      const discs = new Set(tracks.map((t) => t.discNumber));
      const noDisc = vals.filter((x) => !x.v.discNumber).map((x) => x.t.id);
      if (discs.size > 1 && noDisc.length)
        add(
          {
            rule: "discTags",
            severity: "warning",
            trackIds: noDisc,
            what: `${noDisc.length} songs on this multi-disc album without a disc number or disc total`,
            why: "Without them, songs from different discs sort together and players can't show the discs.",
          },
          "discNumber",
        );
      const noIsrc = vals.filter((x) => x.v.archiveVersion === "released" && !x.v.isrc).map((x) => x.t.id);
      if (noIsrc.length)
        add(
          {
            rule: "missingIsrc",
            severity: "info",
            trackIds: noIsrc,
            what: `${noIsrc.length === 1 ? "1 song" : `${noIsrc.length} songs`} released without an ISRC`,
            why: "An ISRC lets other services find the exact recording. Look up ISRCs can fill them from free sources.",
          },
          "isrc",
        );
    }
    const order = { error: 0, warning: 1, info: 2 };
    findings.sort((x, y) => order[x.severity] - order[y.severity]);
    return { findings, styles: [], albums: albums.length, files: total, ignored, fromFiles: folder };
  }

  checkFine(id: string, fine: boolean) {
    if (fine) this.ignored.add(id);
    else this.ignored.delete(id);
  }

  // ---------------------------------------------------------------- fixes

  private change(f: TagEditFile, field: TagField, after: string, note: string | null = null): TagFixChange {
    return { trackId: f.trackId, fileName: f.fileName, stamp: f.stamp ?? "", field, before: f.values[field] ?? null, after, note };
  }

  private preview(kind: TagFixKind, label: string, changes: TagFixChange[], kept: TagFixPreview["kept"], pickEach = false): TagFixPreview {
    const c = changes.length;
    const message =
      c === 0 && kept.length === 0
        ? "Nothing to fill: every song already has it."
        : c === 0
          ? `Nothing to fill. ${files(kept.length)} kept as they are.`
          : `${c} ${c === 1 ? "change" : "changes"} to make.${kept.length ? ` ${files(kept.length)} kept as they are.` : ""}`;
    return { kind, label, changes, kept, pickEach, message };
  }

  private opened(albumId: string): TagEditorData {
    const d = this.open(albumId, []);
    if (d.needsMusicFolder) throw { kind: "invalid", message: NEEDS_FOLDER };
    return d;
  }

  fixPreview(kind: TagFixKind, albumId: string): TagFixPreview {
    const d = this.opened(albumId);
    const readable = d.files.filter((f) => f.stamp);
    if (kind === "discTags") {
      const multi = new Set((this.fx.tracksByAlbum.get(albumId) ?? []).map((t) => t.discNumber)).size > 1;
      const need = readable.filter((f) => !f.values.discNumber || !f.values.discTotal);
      if (multi)
        return this.preview(kind, `Filled disc tags on ${d.title}`, [], need.map((f) => ({ trackId: f.trackId, fileName: f.fileName, reason: "This album has more than one disc. Set its disc numbers in the editor." })));
      const changes = need.flatMap((f) => [
        ...(f.values.discNumber ? [] : [this.change(f, "discNumber", "1")]),
        ...(f.values.discTotal ? [] : [this.change(f, "discTotal", "1")]),
      ]);
      return this.preview(kind, `Filled disc tags on ${d.title}`, changes, []);
    }
    if (kind === "releaseStatus") {
      const counts = new Map<string, number>();
      for (const f of readable) if (f.values.archiveVersion) counts.set(f.values.archiveVersion, (counts.get(f.values.archiveVersion) ?? 0) + 1);
      const best = [...counts.entries()].sort((a, b) => b[1] - a[1])[0];
      const missing = readable.filter((f) => !f.values.archiveVersion);
      const changes = best ? missing.map((f) => this.change(f, "archiveVersion", best[0], `Like the other ${best[1]} songs on disc 1`)) : [];
      const kept = best ? [] : missing.map((f) => ({ trackId: f.trackId, fileName: f.fileName, reason: "No other song on disc 1 has a status to go by." }));
      return this.preview(kind, `Filled release status on ${d.title}`, changes, kept, true);
    }
    throw { kind: "invalid", message: "That fix needs more to go on." };
  }

  isrcReleases(albumId: string): IsrcReleases {
    const d = this.opened(albumId);
    const wanted = d.files.filter((f) => f.stamp && f.values.archiveVersion === "released" && !f.values.isrc).length;
    if (wanted === 0) return { candidates: [], wanted, message: "No released song on this album is missing an ISRC." };
    if (this.lookupDown) throw { kind: "network", message: LOOKUP_DOWN };
    const a = this.fx.albums.find((x) => x.id === albumId)!;
    const release = (source: IsrcRelease["source"], id: string, country: string | null): IsrcRelease => ({
      source,
      id,
      title: a.title,
      artist: a.artistTitle,
      date: a.year ? String(a.year) : null,
      country,
      tracks: d.files.length,
    });
    return { candidates: [release("musicbrainz", "11111111-2222-3333-4444-555555555555", "XW"), release("deezer", "302127", null)], wanted, message: "Pick the release these files are from." };
  }

  isrcPreview(albumId: string, release: IsrcRelease): TagFixPreview {
    if (this.lookupDown) throw { kind: "network", message: LOOKUP_DOWN };
    const d = this.opened(albumId);
    const want = d.files.filter((f) => f.stamp && f.values.archiveVersion === "released" && !f.values.isrc);
    const changes: TagFixChange[] = [];
    const kept: TagFixPreview["kept"] = [];
    want.forEach((f, i) => {
      // Every third song is a re-edit of a different length.
      if (i % 3 === 2) kept.push({ trackId: f.trackId, fileName: f.fileName, reason: "Its length (3:58) differs from the release's (3:41): likely another edit." });
      else changes.push(this.change(f, "isrc", `QZDA${String(41 + (i % 50)).padStart(2, "0")}${String(1000000 + i).slice(-7)}`.slice(0, 12), `${f.values.title} on "${release.title}"`));
    });
    return this.preview("isrc", `Added ISRCs on ${d.title}`, changes, kept);
  }

  explicitPreview(albumIds: string[], explicit: boolean, era?: { artistId: string; era: string } | null): TagFixPreview {
    const ids = era ? this.fx.albums.filter((a) => a.artistId === era.artistId).slice(0, 2).map((a) => a.id) : albumIds;
    if (!ids.length) throw { kind: "notFound", message: "No albums to mark." };
    const want = explicit ? "explicit" : "clean";
    const changes: TagFixChange[] = [];
    let title = "";
    for (const id of ids) {
      const d = this.opened(id);
      title ||= d.title;
      for (const f of d.files) if (f.stamp && f.values.explicit !== want) changes.push(this.change(f, "explicit", want));
    }
    if (ids.length > 1) title = `${ids.length} albums`;
    const p = this.preview("explicit", "", changes, []);
    return { ...p, label: `Marked ${title} as ${want}` };
  }

  // ---------------------------------------------------------------- music folder and Plex

  folderCheck(local: string): MusicFolderCheck {
    if (this.folderGone || !local.trim())
      return {
        ok: false,
        found: 0,
        checked: 5,
        message: "Platter can't open this folder. If it's on a network share, check that the share is connected.",
        mapping: null,
        probes: [],
      };
    return { ok: true, found: 5, checked: 5, message: "Connected, 5 of 5 files found", mapping: { server: SERVER_ROOT, local }, probes: [] };
  }

  setFolder(local: string): MusicFolderCheck {
    const c = this.folderCheck(local);
    if (c.mapping) this.setSettings({ musicFolder: c.mapping.local, musicFolderServer: c.mapping.server });
    return c;
  }

  status(): MusicFolderCheck | null {
    return this.hasFolder() ? this.folderCheck(this.settings().musicFolder) : null;
  }

  librarySettings(): PlexLibraryCheck[] {
    return [
      {
        id: "1",
        title: "Music",
        agent: "Plex Music",
        ok: false,
        items: [
          {
            label: "Local metadata preferred",
            ok: false,
            help: "Plex may replace your tags with its own matches. In Plex, open Music, choose Edit Library, then Advanced, turn on Prefer local metadata and save.",
          },
          { label: "Embedded tags read", ok: true, help: "Plex reads the tags inside your files when it scans them." },
          {
            label: "Personal Media agent available",
            ok: true,
            help: "For an album MusicBrainz doesn't know, choose Fix Match in Plex and pick Personal Media, so a wrong match never overwrites an unreleased album.",
          },
        ],
      },
    ];
  }
}
