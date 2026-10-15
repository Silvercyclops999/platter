/**
 * Mock of editing in Plex (docs/IPC.md "Edit in Plex"): covers and photos,
 * matches, details with locks, merge and split, refresh and undo, on the
 * fixture library. Test hooks: `shared` (this account doesn't own the
 * server) and `failNext` (the next write fails the way Plex would).
 */
import type { EditAccess, EditAction, EditChange, EditFields, EditInfo, EditKind, ImageChoice, ImageKind, MatchCandidate, Settings } from "../types";
import type { Fixtures } from "./fixtures";
import { mockMatched, mockUnreleased } from "./fixtures";

export const NO_PERMISSION = "Only the server's owner and admins can edit this.";
const IMAGE_MAX_BYTES = 10 * 1024 * 1024;

type Failure = "permission" | "offline" | "tooLarge";

interface Extra {
  sortTitle: string;
  released: string;
  studio: string;
  summary: string;
  styles: string[];
  moods: string[];
  locked: string[];
  guid: string;
  art: string | null;
}

export class MockPlexEdit {
  /** This account doesn't own the server: nothing can be edited. */
  shared = false;
  private fail: Failure | null = null;
  private extra = new Map<string, Extra>();
  private log: { change: EditChange; undo: (() => void) | null }[] = [];
  private seq = 0;

  constructor(
    private fx: Fixtures,
    private settings: () => Settings,
    private changed: () => void,
  ) {}

  /** The next write fails: no permission (403), the server offline, or an image too large (413). */
  failNext(f: Failure) {
    this.fail = f;
  }

  access(): EditAccess {
    const s = this.settings();
    return {
      show: s.plexEditShow !== false,
      servers: [],
      connected: !this.shared,
      lockFields: s.plexEditLock !== false,
      matchUnreleased: s.plexEditMatchUnreleased !== false,
    };
  }

  private guard() {
    if (this.settings().plexEditShow === false) throw { kind: "invalid", message: "Editing in Plex is turned off in Settings." };
    if (this.shared) throw { kind: "auth", message: NO_PERMISSION };
  }

  private write() {
    this.guard();
    const f = this.fail;
    this.fail = null;
    if (f === "permission") throw { kind: "auth", message: NO_PERMISSION };
    if (f === "offline") throw { kind: "network", message: "Your server is offline." };
    if (f === "tooLarge") throw { kind: "invalid", message: "That image is too large for the server." };
  }

  private find(id: string): { kind: EditKind; album?: Fixtures["albums"][number]; artist?: Fixtures["artists"][number] } {
    const album = this.fx.albums.find((a) => a.id === id);
    if (album) return { kind: "album", album };
    const artist = this.fx.artists.find((a) => a.id === id);
    if (artist) return { kind: "artist", artist };
    throw { kind: "notFound", message: "That item is gone from the server." };
  }

  private ex(id: string): Extra {
    let e = this.extra.get(id);
    if (!e) {
      const { album } = this.find(id);
      e = {
        sortTitle: "",
        released: album?.year ? `${album.year}` : "",
        studio: album && mockMatched(album) ? "Mock Records" : "",
        summary: "",
        styles: [],
        moods: album ? ["Warm"] : [],
        locked: [],
        guid: album ? (mockMatched(album) ? `plex://album/${id}` : `local://${id}`) : `plex://artist/${id}`,
        art: null,
      };
      this.extra.set(id, e);
    }
    return e;
  }

  private fields(id: string): EditFields {
    const it = this.find(id);
    const e = this.ex(id);
    return {
      title: it.album?.title ?? it.artist!.title,
      sortTitle: e.sortTitle,
      released: e.released,
      studio: e.studio,
      summary: e.summary,
      genres: it.album ? [...it.album.genres] : [],
      styles: [...e.styles],
      moods: [...e.moods],
    };
  }

  item(id: string): EditInfo {
    this.guard();
    const it = this.find(id);
    const e = this.ex(id);
    const s = this.settings();
    return {
      id,
      kind: it.kind,
      fields: this.fields(id),
      locked: [...e.locked],
      guid: e.guid,
      thumb: it.album?.thumb ?? it.artist!.thumb,
      art: e.art ?? it.artist?.art ?? null,
      artistTitle: it.album?.artistTitle ?? null,
      sectionId: "7",
      matched: !e.guid.startsWith("local://"),
      unreleased: !!it.album && mockUnreleased(it.album, s.countUnmatchedUnreleased !== false),
    };
  }

  images(id: string, which: ImageKind): ImageChoice[] {
    this.guard();
    const it = this.find(id);
    const current = which === "art" ? (this.ex(id).art ?? it.artist?.art ?? null) : (it.album?.thumb ?? it.artist!.thumb);
    const providers = it.kind === "album" ? ["caa", "caa", "lastfm", "discogs", "discogs"] : ["fanarttv", "fanarttv", "lastfm", "discogs", "tivo"];
    const out: ImageChoice[] = current ? [{ key: `metadata://${which}s/current-${id}`, preview: current, provider: "embedded", selected: true }] : [];
    providers.forEach((p, i) => {
      const preview = `/library/metadata/${id}/file/${which}-${i + 1}`;
      if (preview !== current) out.push({ key: `https://images.example/${id}/${which}-${i + 1}.jpg`, preview, provider: p, selected: false });
    });
    return out;
  }

  private setImage(id: string, which: ImageKind, path: string | null) {
    const it = this.find(id);
    if (which === "art") this.ex(id).art = path;
    else if (it.album) {
      it.album.thumb = path;
      for (const t of this.fx.tracksByAlbum.get(id) ?? []) t.thumb = path;
    } else it.artist!.thumb = path;
  }

  private imageChange(id: string, which: ImageKind, next: string, label: string): EditChange {
    const it = this.find(id);
    const before = which === "art" ? this.ex(id).art : (it.album?.thumb ?? it.artist!.thumb);
    this.setImage(id, which, next);
    return this.record(id, it.kind, "image", label, () => this.setImage(id, which, before));
  }

  private noun(kind: EditKind, which: ImageKind) {
    return kind === "album" ? "cover" : which === "art" ? "background" : "photo";
  }

  selectImage(id: string, kind: EditKind, which: ImageKind, key: string): EditChange {
    this.write();
    const choice = this.images(id, which).find((c) => c.key === key);
    if (!choice?.preview) throw { kind: "invalid", message: "No image chosen." };
    return this.imageChange(id, which, choice.preview, `Changed the ${this.noun(kind, which)}`);
  }

  uploadImage(id: string, kind: EditKind, which: ImageKind, bytes: Uint8Array, contentType: string): EditChange {
    if (bytes.length > IMAGE_MAX_BYTES) throw { kind: "invalid", message: "Choose an image under 10 MB." };
    if (!["image/jpeg", "image/png", "image/webp"].includes(contentType)) throw { kind: "invalid", message: "Choose a JPEG, PNG or WebP image." };
    this.write();
    return this.imageChange(id, which, `/library/metadata/${id}/upload/${++this.seq}`, `Uploaded a new ${this.noun(kind, which)}`);
  }

  imageLink(id: string, kind: EditKind, which: ImageKind, url: string): EditChange {
    if (!/^https?:\/\/[^/\s]+/.test(url.trim())) throw { kind: "invalid", message: "Paste a link that starts with https://." };
    this.write();
    return this.imageChange(id, which, `/library/metadata/${id}/link/${++this.seq}`, `Set the ${this.noun(kind, which)} from a link`);
  }

  matches(id: string, title?: string, year?: number | null): MatchCandidate[] {
    this.guard();
    const it = this.find(id);
    const name = title?.trim() || it.album?.title || it.artist!.title;
    const y = year ?? it.album?.year ?? null;
    if (it.kind === "artist") {
      return [
        { guid: `mbid://mock-artist-${id}-1`, name, artist: "United Kingdom", year: null, score: 100 },
        { guid: `mbid://mock-artist-${id}-2`, name: `${name} Trio`, artist: "Germany", year: null, score: 62 },
      ];
    }
    return [
      { guid: `mbid://mock-${id}-1`, name, artist: it.album!.artistTitle, year: y, score: 98 },
      { guid: `mbid://mock-${id}-2`, name: `${name} (Deluxe Edition)`, artist: it.album!.artistTitle, year: y ? y + 1 : null, score: 86 },
      { guid: `mbid://mock-${id}-3`, name: `${name} Live`, artist: "Various Artists", year: y ? y + 3 : null, score: 41 },
    ];
  }

  match(id: string, kind: EditKind, c: MatchCandidate): EditChange {
    this.write();
    const it = this.find(id);
    if (it.album && this.settings().plexEditMatchUnreleased === false && mockUnreleased(it.album, this.settings().countUnmatchedUnreleased !== false)) {
      throw { kind: "invalid", message: "Fix match is turned off for unreleased albums in Settings." };
    }
    const e = this.ex(id);
    const before = e.guid;
    e.guid = c.guid;
    return this.record(id, kind, "match", `Matched to ${c.name}`, () => (e.guid = before));
  }

  unmatch(id: string, kind: EditKind): EditChange {
    this.write();
    const e = this.ex(id);
    const before = e.guid;
    const was = !before.startsWith("local://");
    e.guid = `local://${id}`;
    return this.record(id, kind, "unmatch", "Unmatched", was ? () => (e.guid = before) : null);
  }

  private apply(id: string, f: EditFields, locks: string[]) {
    const it = this.find(id);
    const e = this.ex(id);
    if (it.album) {
      it.album.title = f.title;
      it.album.genres = [...f.genres];
      const y = Number.parseInt(f.released.slice(0, 4), 10);
      it.album.year = Number.isFinite(y) ? y : null;
    } else it.artist!.title = f.title;
    Object.assign(e, { sortTitle: f.sortTitle, released: f.released, studio: f.studio, summary: f.summary, styles: [...f.styles], moods: [...f.moods] });
    e.locked = locks;
  }

  details(id: string, f: EditFields): EditChange {
    this.write();
    if (!f.title.trim()) throw { kind: "invalid", message: "The title can't be empty." };
    if (f.released.trim() && !/^\d{4}(-\d{2}-\d{2})?$/.test(f.released.trim())) {
      throw { kind: "invalid", message: "Use a year like 2021 or a date like 2021-06-25." };
    }
    const it = this.find(id);
    const before = this.fields(id);
    const beforeLocks = [...this.ex(id).locked];
    const map: [keyof EditFields, string, string][] = [
      ["title", "title", "the title"],
      ["sortTitle", "titleSort", "the sort title"],
      ["released", "originallyAvailableAt", "the release date"],
      ["studio", "studio", "the record label"],
      ["summary", "summary", "the summary"],
      ["genres", "genre", "the genres"],
      ["styles", "style", "the styles"],
      ["moods", "mood", "the moods"],
    ];
    const changed = map.filter(([k]) => JSON.stringify(before[k]) !== JSON.stringify(f[k]));
    if (!changed.length) throw { kind: "invalid", message: "Nothing changed." };
    const locks = this.settings().plexEditLock !== false ? [...new Set([...beforeLocks, ...changed.map(([, p]) => p)])] : beforeLocks;
    this.apply(id, f, locks);
    const label = changed.length === 1 ? `Edited ${changed[0][2]}` : `Edited ${changed.length} details`;
    return this.record(id, it.kind, "details", label, () => this.apply(id, before, beforeLocks));
  }

  merge(target: string, others: string[], photoFrom?: string | null): EditChange {
    this.write();
    const keep = this.fx.artists.find((a) => a.id === target);
    if (!keep) throw { kind: "notFound", message: "That artist is gone from the server." };
    if (!others.length || others.includes(target)) throw { kind: "invalid", message: "Pick another artist to merge with." };
    const gone = this.fx.artists.filter((a) => others.includes(a.id));
    const moved = this.fx.albums.filter((a) => others.includes(a.artistId)).map((a) => ({ a, artistId: a.artistId, artistTitle: a.artistTitle }));
    const thumb = keep.thumb;
    const photo = photoFrom && photoFrom !== target ? gone.find((a) => a.id === photoFrom)?.thumb : undefined;
    if (photo !== undefined) keep.thumb = photo;
    this.fx.artists = this.fx.artists.filter((a) => !others.includes(a.id));
    for (const { a } of moved) {
      a.artistId = target;
      a.artistTitle = keep.title;
    }
    const label = others.length === 1 ? `Merged another artist into ${keep.title}` : `Merged ${others.length} artists into ${keep.title}`;
    return this.record(target, "artist", "merge", label, () => {
      keep.thumb = thumb;
      this.fx.artists = [...this.fx.artists, ...gone];
      for (const m of moved) {
        m.a.artistId = m.artistId;
        m.a.artistTitle = m.artistTitle;
      }
    });
  }

  /** Merge albums shown apart into `target`: their songs move over. */
  mergeAlbums(target: string, others: string[]): EditChange {
    this.write();
    const keep = this.fx.albums.find((a) => a.id === target);
    if (!keep) throw { kind: "notFound", message: "That album is gone from the server." };
    if (!others.length || others.includes(target)) throw { kind: "invalid", message: "Pick another album to merge with." };
    const gone = this.fx.albums.filter((a) => others.includes(a.id));
    const moved = this.fx.tracks.filter((t) => others.includes(t.albumId)).map((t) => ({ t, albumId: t.albumId, albumTitle: t.albumTitle }));
    const before = new Map(this.fx.tracksByAlbum);
    const count = keep.trackCount;
    for (const { t } of moved) {
      t.albumId = target;
      t.albumTitle = keep.title;
    }
    this.fx.tracksByAlbum.set(target, [...(this.fx.tracksByAlbum.get(target) ?? []), ...moved.map((m) => m.t)]);
    for (const o of others) this.fx.tracksByAlbum.delete(o);
    this.fx.albums = this.fx.albums.filter((a) => !others.includes(a.id));
    keep.trackCount = count + moved.length;
    const label = others.length === 1 ? `Merged the two parts of ${keep.title}` : `Merged ${others.length + 1} parts of ${keep.title}`;
    return this.record(target, "album", "merge", label, () => {
      for (const m of moved) {
        m.t.albumId = m.albumId;
        m.t.albumTitle = m.albumTitle;
      }
      this.fx.tracksByAlbum.clear();
      for (const [k, v] of before) this.fx.tracksByAlbum.set(k, v);
      this.fx.albums = [...this.fx.albums, ...gone];
      keep.trackCount = count;
    });
  }

  split(id: string): EditChange {
    this.write();
    this.find(id);
    return this.record(id, "artist", "split", "Split", null);
  }

  refresh(id: string, kind: EditKind): EditChange {
    this.write();
    this.find(id);
    return this.record(id, kind, "refresh", "Refreshed details", null);
  }

  undo(changeId: string): EditChange {
    const e = this.log.find((l) => l.change.id === changeId);
    if (!e) throw { kind: "notFound", message: "That change isn't in the list any more." };
    if (e.change.undone) throw { kind: "invalid", message: "That change was already undone." };
    if (!e.undo) throw { kind: "invalid", message: "That change can't be undone." };
    this.write();
    e.undo();
    e.change.undone = true;
    this.changed();
    return { ...e.change };
  }

  history(id: string): EditChange[] {
    return this.log
      .filter((l) => l.change.itemId === id)
      .map((l) => ({ ...l.change }))
      .reverse();
  }

  private record(itemId: string, kind: EditKind, action: EditAction, label: string, undo: (() => void) | null): EditChange {
    const change: EditChange = { id: `c${++this.seq}`, itemId, kind, action, label, at: Date.now(), undoable: !!undo, undone: false };
    this.log.push({ change, undo });
    this.changed();
    return { ...change };
  }
}
