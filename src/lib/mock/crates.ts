/**
 * Crates for the mock backend, worked out like the core's: each album's
 * label, producers and recording studio from its (made-up) sleeve notes,
 * its decade and year, and its genres. Names fold so spellings share a
 * crate; a crate holds two albums at least.
 */
import type { Album, AlbumShelf, Crate, CrateKind, CrateList, CratePage, Settings } from "../types";
import type { Fixtures } from "./fixtures";
import type { MockSleeve } from "./sleeve";

const PREVIEW = 12;
const SHELF_N = 12;
const MIN_ALBUMS = 2;
const LABEL_TAIL = ["records", "recordings", "record", "music", "entertainment", "inc", "ltd", "llc", "limited", "co", "label", "group"];
const STUDIO_TAIL = ["studios", "studio", "recording", "recordings"];

function foldWith(name: string, tail: string[]): string {
  const words = name
    .toLowerCase()
    .split(/[^\p{L}\p{N}]+/u)
    .filter(Boolean);
  if (words[0] === "the" && words.length > 1) words.shift();
  while (words.length > 1 && tail.includes(words[words.length - 1])) words.pop();
  return words.join(" ");
}

interface Entry {
  kind: CrateKind;
  key: string;
  name: string;
  ids: string[];
}

export class MockCrates {
  constructor(
    private fx: Fixtures,
    private sleeve: MockSleeve,
    private settings: () => Settings,
    private onMap: (trackIds: string[]) => boolean,
  ) {}

  private index(): Entry[] {
    const names = new Map<string, Map<string, number>>();
    const albums = new Map<string, Set<string>>();
    const add = (kind: CrateKind, key: string, name: string, id: string) => {
      if (!key) return;
      const k = `${kind}\u0000${key}`;
      const n = names.get(k) ?? new Map<string, number>();
      n.set(name, (n.get(name) ?? 0) + 1);
      names.set(k, n);
      albums.set(k, (albums.get(k) ?? new Set()).add(id));
    };
    for (const a of this.fx.albums) {
      const notes = this.sleeve.notes(a, true);
      if (notes.label) add("label", foldWith(notes.label.value, LABEL_TAIL), notes.label.value, a.id);
      for (const p of notes.groups.find((g) => g.role === "producer")?.people ?? []) add("producer", p.key, p.name, a.id);
      for (const s of notes.studios) add("studio", foldWith(s.name, STUDIO_TAIL), s.name, a.id);
      if (a.year) {
        const decade = a.year - (a.year % 10);
        add("era", String(decade), `${decade}s`, a.id);
        add("year", String(a.year), String(a.year), a.id);
      }
      for (const g of a.genres) add("genre", g.toLowerCase(), g, a.id);
    }
    const byId = new Map(this.fx.albums.map((a) => [a.id, a]));
    const order = (x: string, y: string) => {
      const a = byId.get(x)!;
      const b = byId.get(y)!;
      return (a.year || 9999) - (b.year || 9999) || a.artistTitle.localeCompare(b.artistTitle) || a.sortTitle.localeCompare(b.sortTitle);
    };
    const out: Entry[] = [];
    for (const [k, ids] of albums) {
      if (ids.size < MIN_ALBUMS) continue;
      const [kind, key] = k.split("\u0000") as [CrateKind, string];
      const spellings = [...names.get(k)!.entries()].sort((a, b) => b[1] - a[1] || a[0].length - b[0].length || a[0].localeCompare(b[0]));
      out.push({ kind, key, name: spellings[0][0], ids: [...ids].sort(order) });
    }
    return out;
  }

  private albums(ids: string[]): Album[] {
    const byId = new Map(this.fx.albums.map((a) => [a.id, a]));
    return ids.map((id) => byId.get(id)).filter((a): a is Album => !!a);
  }

  private span(ids: string[]): { fromYear: number | null; toYear: number | null; artists: number } {
    const list = this.albums(ids);
    const years = list.map((a) => a.year).filter((y): y is number => !!y);
    return {
      fromYear: years.length ? Math.min(...years) : null,
      toYear: years.length ? Math.max(...years) : null,
      artists: new Set(list.map((a) => a.artistId)).size,
    };
  }

  list(kind: CrateKind): CrateList {
    if (this.settings().showCrates === false) return { kind, crates: [] };
    const entries = this.index().filter((e) => e.kind === kind);
    if (kind === "era" || kind === "year") entries.sort((a, b) => b.key.localeCompare(a.key));
    else entries.sort((a, b) => b.ids.length - a.ids.length || a.name.toLowerCase().localeCompare(b.name.toLowerCase()));
    const crates: Crate[] = entries.map((e) => ({ kind, key: e.key, name: e.name, count: e.ids.length, ...this.span(e.ids), albums: this.albums(e.ids.slice(0, PREVIEW)) }));
    return { kind, crates };
  }

  page(kind: CrateKind, key: string): CratePage | null {
    if (this.settings().showCrates === false) return null;
    const all = this.index();
    const e = all.find((x) => x.kind === kind && x.key === key);
    if (!e) return null;
    const years =
      kind === "era"
        ? all
            .filter((x) => x.kind === "year" && Number(x.key) - (Number(x.key) % 10) === Number(key))
            .map((x) => Number(x.key))
            .sort((a, b) => a - b)
        : [];
    return {
      kind,
      key: e.key,
      name: e.name,
      ...this.span(e.ids),
      albums: this.albums(e.ids).map((album) => ({ album, onMap: this.onMap((this.fx.tracksByAlbum.get(album.id) ?? []).map((t) => t.id)) })),
      years,
      trackIds: e.ids.flatMap((id) => (this.fx.tracksByAlbum.get(id) ?? []).map((t) => t.id)).slice(0, 5000),
    };
  }

  shelves(albumId: string): AlbumShelf[] {
    const s = this.settings();
    if (s.showCrates === false || s.crateShelves === false) return [];
    const album = this.fx.albums.find((a) => a.id === albumId);
    const rank: CrateKind[] = ["label", "producer", "studio"];
    return this.index()
      .filter((e) => rank.includes(e.kind) && e.ids.includes(albumId))
      .sort((a, b) => rank.indexOf(a.kind) - rank.indexOf(b.kind) || b.ids.length - a.ids.length)
      .map((e) => {
        const others = this.albums(e.ids.filter((id) => id !== albumId));
        // Other artists first.
        others.sort((a, b) => Number(a.artistId === album?.artistId) - Number(b.artistId === album?.artistId));
        return { kind: e.kind, key: e.key, name: e.name, count: e.ids.length, albums: others.slice(0, SHELF_N) };
      })
      .filter((x) => x.albums.length > 0);
  }
}
