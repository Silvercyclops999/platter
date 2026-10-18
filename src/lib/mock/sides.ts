/**
 * Record sides for the mock: albums of four songs or more (not the demos)
 * whose number isn't a multiple of three have a vinyl edition, split in
 * half, or in four when there are twelve songs or more. Splits by hand win.
 */
import type { Fixtures } from "./fixtures";
import type { AlbumSide, AlbumSides } from "../types";

const label = (i: number) => String.fromCharCode(65 + i);

function split(ids: string[], parts: number): AlbumSide[] {
  const size = Math.ceil(ids.length / parts);
  const out: AlbumSide[] = [];
  for (let i = 0; i < ids.length; i += size) out.push({ label: label(out.length), trackIds: ids.slice(i, i + size) });
  return out;
}

export class MockSides {
  /** Splits by hand: album id to the songs sides B, C... start on. */
  private manual = new Map<string, string[]>();

  constructor(private fx: Fixtures) {}

  reset() {
    this.manual.clear();
  }

  private ids(albumId: string): string[] {
    return (this.fx.tracksByAlbum.get(albumId) ?? []).map((t) => t.id);
  }

  private detected(albumId: string): AlbumSide[] | null {
    const album = this.fx.albums.find((a) => a.id === albumId);
    const ids = this.ids(albumId);
    if (!album || album.section === "demos" || ids.length < 4) return null;
    const n = Number(album.id) || album.title.length;
    if (n % 3 === 0) return null;
    return split(ids, ids.length >= 12 ? 4 : 2);
  }

  get(albumId: string): AlbumSides {
    const ids = this.ids(albumId);
    const detected = this.detected(albumId);
    const starts = this.manual.get(albumId);
    if (starts) {
      const sides: AlbumSide[] = [];
      ids.forEach((id, i) => {
        if (i === 0 || starts.includes(id)) sides.push({ label: label(sides.length), trackIds: [] });
        sides[sides.length - 1].trackIds.push(id);
      });
      return { albumId, sides: starts.length && sides.length > 1 ? sides : [], source: "manual", detected: !!detected };
    }
    return detected
      ? { albumId, sides: detected, source: "musicbrainz", detected: true }
      : { albumId, sides: [], source: null, detected: false };
  }

  set(albumId: string, starts: string[] | null): AlbumSides {
    const ids = this.ids(albumId);
    if (starts === null) this.manual.delete(albumId);
    else {
      if (starts.some((s) => !ids.includes(s))) throw new Error("That song isn't on this album.");
      if (starts.includes(ids[0])) throw new Error("Side A starts on the first song.");
      this.manual.set(
        albumId,
        ids.filter((id) => starts.includes(id)),
      );
    }
    return this.get(albumId);
  }
}
