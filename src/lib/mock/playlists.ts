/**
 * Playlist editing for the mock backend: the same rules as the core
 * (src-tauri/src/playlists.rs), applied to the fixture playlists in memory.
 * Writes happen at once, as if the server were always reachable; the test
 * hooks fake Plex refusing a write and a playlist deleted on another device.
 */
import type { AddPlace, Playlist, RemovedItem, Track } from "../types";
import type { Fixtures } from "./fixtures";

export const DEFAULT_PLAYLIST_NAME = "My playlist #N";
const TITLE_MAX = 100;
const SUMMARY_MAX = 300;

interface Row {
  itemId: string;
  trackId: string;
}

const invalid = (message: string) => ({ kind: "invalid", message });
const notFound = (message: string) => ({ kind: "notFound", message });

/** The next free "#N" name from a template (see db::playlists::next_name). */
export function nextPlaylistName(titles: string[], template: string): string {
  const t = template.trim() || DEFAULT_PLAYLIST_NAME;
  const at = t.indexOf("#N");
  if (at < 0) return t;
  const before = t.slice(0, at);
  const after = t.slice(at + 2);
  const used = new Set<number>();
  for (const title of titles) {
    if (!title.startsWith(`${before}#`) || !title.endsWith(after)) continue;
    const n = Number(title.slice(before.length + 1, title.length - after.length));
    if (Number.isInteger(n) && n > 0) used.add(n);
  }
  let n = 1;
  while (used.has(n)) n++;
  return `${before}#${n}${after}`;
}

export class MockPlaylists {
  private rows = new Map<string, Row[]>();
  private seq = 1;
  private nextId = 900;
  private refuseNext = false;

  constructor(
    private fx: Fixtures,
    private track: (id: string) => Track | undefined,
    /** Library changed: pages and the sidebar refetch. */
    private changed: () => void,
    private toastError: (message: string) => void,
  ) {
    const now = Math.floor(Date.now() / 1000);
    fx.playlists.forEach((p, i) => {
      this.rows.set(p.id, (fx.playlistTracks.get(p.id) ?? []).map((trackId) => ({ itemId: this.item(), trackId })));
      p.summary ??= null;
      p.updatedAt ??= now - (i + 1) * 86_400;
    });
  }

  private item(): string {
    return `it${this.seq++}`;
  }

  itemIds(id: string): string[] {
    return (this.rows.get(id) ?? []).map((r) => r.itemId);
  }

  private find(id: string): Playlist {
    const p = this.fx.playlists.find((x) => x.id === id);
    if (!p) throw notFound("Playlist not found");
    return p;
  }

  private editable(id: string): Playlist {
    const p = this.find(id);
    if (p.smart) throw invalid("Smart playlists can't be changed here.");
    return p;
  }

  /** Recount after a change and keep the fixture track lists in step. */
  private touch(p: Playlist) {
    const rows = this.rows.get(p.id) ?? [];
    this.fx.playlistTracks.set(p.id, rows.map((r) => r.trackId));
    p.trackCount = rows.length;
    p.durationMs = rows.reduce((s, r) => s + (this.track(r.trackId)?.durationMs ?? 0), 0);
    p.updatedAt = Math.max(Math.floor(Date.now() / 1000), (p.updatedAt ?? 0) + 1);
  }

  /** Apply a write; a refused one is undone a moment later with a toast. */
  private write<T>(id: string, fn: () => T, what: "change" | "create" | "delete" = "change"): T {
    const refuse = this.refuseNext;
    this.refuseNext = false;
    const before = refuse ? this.snapshot() : null;
    const out = fn();
    this.changed();
    if (before) {
      setTimeout(() => {
        const name = before.playlists.find((p) => p.id === id)?.title ?? this.fx.playlists.find((p) => p.id === id)?.title;
        this.restore(before);
        const q = name ? `"${name}"` : "a playlist";
        this.toastError(
          what === "delete" ? `Plex didn't delete ${q}.` : what === "create" ? `Plex didn't accept the new playlist ${q}.` : `Plex didn't accept a change to ${q}. Showing it as it is in Plex.`,
        );
        this.changed();
      }, 0);
    }
    return out;
  }

  private snapshot() {
    return {
      playlists: this.fx.playlists.map((p) => ({ ...p })),
      rows: new Map([...this.rows].map(([k, v]) => [k, v.map((r) => ({ ...r }))])),
    };
  }

  private restore(s: ReturnType<MockPlaylists["snapshot"]>) {
    this.fx.playlists.splice(0, this.fx.playlists.length, ...s.playlists);
    this.rows = s.rows;
    this.fx.playlistTracks.clear();
    for (const [k, v] of this.rows) this.fx.playlistTracks.set(k, v.map((r) => r.trackId));
  }

  // ---------------------------------------------------------------- commands

  create(template: string, title: string | undefined, trackIds: string[]): Playlist {
    const name = title === undefined ? nextPlaylistName(this.fx.playlists.map((p) => p.title), template) : title.trim();
    if (!name) throw invalid("Playlist name is required.");
    if ([...name].length > TITLE_MAX) throw invalid(`Playlist names can be up to ${TITLE_MAX} characters.`);
    const id = String(this.nextId++);
    return this.write(
      id,
      () => {
        const p: Playlist = { id, title: name, smart: false, trackCount: 0, durationMs: 0, thumb: null, summary: null, updatedAt: null };
        this.fx.playlists.push(p);
        this.rows.set(id, trackIds.map((trackId) => ({ itemId: this.item(), trackId })));
        this.touch(p);
        return { ...p };
      },
      "create",
    );
  }

  add(id: string, trackIds: string[], place: AddPlace = {}): Playlist {
    if (!trackIds.length) throw invalid("No songs to add.");
    const p = this.editable(id);
    return this.write(id, () => {
      const rows = this.rows.get(id)!;
      const at = place.after ? rows.findIndex((r) => r.itemId === place.after) + 1 || rows.length : place.top ? 0 : rows.length;
      rows.splice(at, 0, ...trackIds.map((trackId) => ({ itemId: this.item(), trackId })));
      this.touch(p);
      return { ...p };
    });
  }

  duplicates(id: string, trackIds: string[]): string[] {
    this.find(id);
    const have = new Set((this.rows.get(id) ?? []).map((r) => r.trackId));
    return [...new Set(trackIds.filter((t) => have.has(t)))];
  }

  remove(id: string, itemIds: string[]): RemovedItem[] {
    const p = this.editable(id);
    return this.write(id, () => {
      const rows = this.rows.get(id)!;
      const gone: RemovedItem[] = [];
      const kept = rows.filter((r, index) => {
        if (!itemIds.includes(r.itemId)) return true;
        gone.push({ itemId: r.itemId, trackId: r.trackId, index });
        return false;
      });
      this.rows.set(id, kept);
      this.touch(p);
      return gone;
    });
  }

  move(id: string, itemIds: string[], after: string | null | undefined): void {
    const p = this.editable(id);
    if (after && itemIds.includes(after)) throw invalid("Can't move songs after one of themselves.");
    this.write(id, () => {
      const rows = this.rows.get(id)!;
      const moving = rows.filter((r) => itemIds.includes(r.itemId));
      const rest = rows.filter((r) => !itemIds.includes(r.itemId));
      const at = after ? rest.findIndex((r) => r.itemId === after) + 1 : 0;
      if (after && at === 0) throw notFound("Playlist item not found");
      rest.splice(at, 0, ...moving);
      this.rows.set(id, rest);
      this.touch(p);
    });
  }

  update(id: string, details: { title?: string; summary?: string }): Playlist {
    const p = this.editable(id);
    const title = details.title?.trim();
    const summary = details.summary?.trim();
    if (title !== undefined && !title) throw invalid("Playlist name is required.");
    if (title && [...title].length > TITLE_MAX) throw invalid(`Playlist names can be up to ${TITLE_MAX} characters.`);
    if (summary && [...summary].length > SUMMARY_MAX) throw invalid(`Descriptions can be up to ${SUMMARY_MAX} characters.`);
    return this.write(id, () => {
      if (title !== undefined) p.title = title;
      if (summary !== undefined) p.summary = summary || null;
      p.updatedAt = Math.floor(Date.now() / 1000);
      return { ...p };
    });
  }

  setCover(id: string, bytes: Uint8Array, contentType: string): void {
    const p = this.editable(id);
    if (!["image/jpeg", "image/png", "image/webp"].includes(contentType)) throw invalid("Choose a JPEG, PNG or WebP image.");
    if (!bytes.length || bytes.length > 10 * 1024 * 1024) throw invalid("Choose an image under 10 MB.");
    this.write(id, () => {
      p.thumb = `/library/metadata/${id}/thumb/${Date.now()}`;
    });
  }

  delete(id: string): void {
    this.editable(id);
    this.write(
      id,
      () => {
        const i = this.fx.playlists.findIndex((p) => p.id === id);
        this.fx.playlists.splice(i, 1);
        this.rows.delete(id);
        this.fx.playlistTracks.delete(id);
      },
      "delete",
    );
  }

  // ---------------------------------------------------------------- test hooks

  /** The next write reaches "Plex", which refuses it. */
  refuseNextWrite() {
    this.refuseNext = true;
  }

  /** Another device deleted this playlist. */
  deleteElsewhere(id: string) {
    const i = this.fx.playlists.findIndex((p) => p.id === id);
    if (i < 0) return;
    this.fx.playlists.splice(i, 1);
    this.rows.delete(id);
    this.fx.playlistTracks.delete(id);
    this.changed();
  }
}
