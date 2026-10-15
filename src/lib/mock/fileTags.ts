/**
 * Mock of the file tag scan (docs/IPC.md "File tags"). Invented tags on the
 * fixture library: a few explicit songs, leak / snippet / session / demo
 * versions on the unreleased builds and demos, a fan edit, a source quality
 * that differs from the file, and one album group with two editions (the two
 * builds of one artist). The scan "reads" files in the background and
 * reports progress like the core.
 */
import type { Album, Settings, TagIndex, TagScanStatus, Track, TrackBadges } from "../types";
import type { Fixtures } from "./fixtures";
import { hashString } from "./fixtures";

/** Index of the artist whose two builds are editions of one album. */
const GROUP_ARTIST = 3;
const GROUP_KEY = "untitled-builds";

export interface MockTagHooks {
  emitProgress: (s: TagScanStatus) => void;
  emitChanged: () => void;
  later: (ms: number, fn: () => void) => void;
}

export class MockFileTags {
  private tracks: Record<string, TrackBadges> = {};
  private albumTags: TagIndex["albums"] = {};
  private groups: TagIndex["groups"] = {};
  private status: TagScanStatus;
  private running = false;

  constructor(
    private fx: Fixtures,
    private settings: () => Settings,
    private hooks: MockTagHooks,
    /** Start with every file read (tests); otherwise the scan runs on start. */
    scanned = true,
  ) {
    this.build();
    const total = fx.tracks.length;
    this.status = { enabled: true, running: false, done: scanned ? total : 0, total, failed: scanned ? 2 : 0, paused: false };
    if (!scanned) this.hooks.later(800, () => this.run());
  }

  private build() {
    const { albums, tracksByAlbum, artists } = this.fx;
    const set = (t: Track, b: TrackBadges) => (this.tracks[t.id] = { ...this.tracks[t.id], ...b });
    // The first album's opening song is explicit (it's in the player bar at start).
    const first = tracksByAlbum.get(albums[0].id)?.[0];
    if (first) set(first, { explicit: true });
    // Hip-hop artists: about one song in three is explicit.
    for (const t of this.fx.tracks) {
      const genre = albums.find((a) => a.id === t.albumId)?.genres[0];
      if (genre === "Hip-Hop" && hashString(`explicit:${t.id}`) % 3 === 0) set(t, { explicit: true });
    }
    for (const a of albums) {
      const list = tracksByAlbum.get(a.id) ?? [];
      if (a.subformats.includes("demo")) list.forEach((t) => set(t, { version: "demo" }));
      if (a.format === null) {
        list.forEach((t, i) => {
          const version = i === list.length - 1 ? "snippet" : i % 4 === 1 ? "session" : "leak";
          set(t, { version, ...(version === "snippet" ? { quality: "lq" } : i === 0 ? { quality: "lossless" } : {}) });
        });
      }
    }
    // A fan edit on the last artist's build.
    const allBuilds = albums.filter((a) => a.format === null);
    const lastBuild = allBuilds.find((a) => a.artistId === artists[artists.length - 1]?.id) ?? allBuilds[allBuilds.length - 1];
    const edit = lastBuild && tracksByAlbum.get(lastBuild.id)?.[1];
    if (edit) set(edit, { edit: true, version: "alternate" });

    for (const a of albums) {
      const list = tracksByAlbum.get(a.id) ?? [];
      const badges = list.map((t) => this.tracks[t.id]).filter(Boolean);
      const explicit = badges.some((b) => b.explicit);
      const version = badges.find((b) => b.version)?.version;
      if (explicit || version) this.albumTags[a.id] = { ...(explicit ? { explicit } : {}), ...(version ? { version } : {}) };
    }
    // One album group with two editions.
    const artistId = artists[GROUP_ARTIST]?.id;
    const builds = albums.filter((a) => a.artistId === artistId && a.format === null).slice(0, 2);
    if (builds.length === 2) {
      const labels = ["Build v1", "Build v2"];
      // Face: no Standard and nothing released, so the earliest added.
      const ordered = [...builds].sort((x, y) => x.addedAt - y.addedAt);
      builds.forEach((a) => {
        const label = labels[ordered.indexOf(a)];
        this.albumTags[a.id] = { ...this.albumTags[a.id], group: GROUP_KEY, edition: label };
      });
      this.groups[GROUP_KEY] = ordered.map((a, i) => ({ albumId: a.id, title: a.title, label: labels[i], year: a.year }));
    }
  }

  /** Badges of songs and albums added after the build (the archive demo artist). */
  addBadges(b: { tracks: Record<string, TrackBadges>; albums: TagIndex["albums"] }) {
    Object.assign(this.tracks, b.tracks);
    Object.assign(this.albumTags, b.albums);
  }

  index(): TagIndex {
    return { tracks: this.tracks, albums: this.albumTags, groups: this.groups };
  }

  scanStatus(): TagScanStatus {
    return { ...this.status, enabled: this.settings().readFileTags !== false };
  }

  /** The settings changed: the scan follows "Read extra tags from files". */
  settingsChanged() {
    this.hooks.emitProgress(this.scanStatus());
    if (this.settings().readFileTags !== false && this.status.done < this.status.total) this.run();
  }

  private run() {
    if (this.running) return;
    this.running = true;
    const step = () => {
      if (this.settings().readFileTags === false) {
        this.running = false;
        this.status = { ...this.status, running: false };
        this.hooks.emitProgress(this.scanStatus());
        return;
      }
      const done = Math.min(this.status.total, this.status.done + 37);
      this.status = { ...this.status, done, running: done < this.status.total, failed: done >= this.status.total ? 2 : 0 };
      this.hooks.emitProgress(this.scanStatus());
      if (done < this.status.total) this.hooks.later(400, step);
      else {
        this.running = false;
        this.hooks.emitChanged();
      }
    };
    step();
  }

  /** Albums the library hides: editions behind their group's face. */
  hiddenAlbums(): Set<string> {
    const out = new Set<string>();
    if (this.settings().groupEditions === false) return out;
    for (const list of Object.values(this.groups)) list.slice(1).forEach((e) => out.add(e.albumId));
    return out;
  }

  visibleAlbums(list: Album[]): Album[] {
    const hidden = this.hiddenAlbums();
    return hidden.size ? list.filter((a) => !hidden.has(a.id)) : list;
  }

  /** Album page songs: fan and AI edits leave unless shown. */
  albumTracks(list: Track[]): Track[] {
    if (this.settings().showFanEdits) return list;
    return list.filter((t) => !this.tracks[t.id]?.edit);
  }

  /** What plays: explicit songs (when filtered) and edits (when hidden) leave. */
  forPlay(list: Track[]): { tracks: Track[]; message: string | null } {
    const s = this.settings();
    let explicit = 0;
    let edits = 0;
    const tracks = list.filter((t) => {
      const b = this.tracks[t.id];
      if (s.explicitContent === "filtered" && b?.explicit) {
        explicit++;
        return false;
      }
      if (!s.showFanEdits && b?.edit) {
        edits++;
        return false;
      }
      return true;
    });
    return { tracks, message: skippedMessage(explicit, edits) };
  }
}

/** Same words as the core's toast. */
export function skippedMessage(explicit: number, edits: number): string | null {
  const e = explicit === 1 ? "1 explicit song" : `${explicit} explicit songs`;
  const f = edits === 1 ? "1 fan or AI edit" : `${edits} fan or AI edits`;
  if (!explicit && !edits) return null;
  if (!edits) return `Skipped ${e}. Explicit content is filtered in Settings.`;
  if (!explicit) return `Skipped ${f}.`;
  return `Skipped ${e} and ${f}.`;
}
