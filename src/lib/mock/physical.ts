/**
 * The physical collection in the browser mock (docs/IPC.md "Physical
 * collection"): a few owned albums from the fixture library, a faked
 * Discogs collection (names made up here, never a real account) with a
 * couple of records the library has and a few it doesn't, and test hooks
 * for Discogs refusing the token, being busy or out of reach.
 */
import type {
  Album,
  DiscogsReport,
  DiscogsStatus,
  OwnedAlbum,
  OwnedCopy,
  PhysicalFormat,
  SetOwned,
  Settings,
  ShelfAlbum,
  ShelfOnly,
  ShelfStats,
} from "../types";
import type { Fixtures } from "./fixtures";

const FORMATS: PhysicalFormat[] = ["vinyl", "cd", "cassette", "other"];
/** A token the faked Discogs accepts (any other non-empty one is accepted too unless `refuse` is set). */
export const MOCK_DISCOGS_USER = "shelfkeeper";

const copy = (format: PhysicalFormat, extra: Partial<OwnedCopy> = {}): OwnedCopy => ({ format, pressing: "", colour: "", condition: "", notes: "", ...extra });

const fold = (s: string) => s.trim().toLowerCase().replace(/\s+/g, " ");

const enc = encodeURIComponent;
const links = (artist: string, title: string) => {
  const q = enc(`${artist} ${title}`);
  return {
    bandcamp: `https://bandcamp.com/search?q=${q}&item_type=a`,
    qobuz: `https://www.qobuz.com/search?q=${q}`,
    itunes: `https://music.apple.com/search?term=${q}`,
  };
};

interface FakeItem {
  releaseId: number;
  artist: string;
  title: string;
  year: number | null;
  format: PhysicalFormat;
  pressing: string;
  colour: string;
  condition: string;
}

export class MockPhysical {
  readonly marks = new Map<string, OwnedAlbum>();
  /** The saved token (the keychain in the app). Never sent back. */
  private token: string | null = null;
  private username: string | null = null;
  private report: DiscogsReport | null = null;
  importing = false;
  /** Test hooks: Discogs refuses the token / answers 429 / can't be reached. */
  refuse = false;
  fail: "busy" | "network" | null = null;
  /** Every token Discogs was asked about (tests check what was sent). */
  readonly checked: string[] = [];

  constructor(
    private fx: Fixtures,
    private settings: () => Settings,
    private emit: () => void,
  ) {
    this.reset();
  }

  private played(): Album[] {
    return [...this.fx.albums].filter((a) => a.format !== null).sort((a, b) => b.viewCount - a.viewCount);
  }

  /** Back to the seeded marks, no token. */
  reset() {
    this.marks.clear();
    this.token = null;
    this.username = null;
    this.report = null;
    this.importing = false;
    this.refuse = false;
    this.fail = null;
    this.checked.length = 0;
    const top = this.played();
    const seed: [Album | undefined, OwnedCopy[]][] = [
      [top[0], [copy("vinyl", { pressing: "LP, Album, Reissue", colour: "Red translucent", condition: "Near mint" })]],
      [top[2], [copy("vinyl"), copy("cd", { notes: "Bought at the show" })]],
      [top[5], [copy("cassette", { condition: "Very good" })]],
    ];
    seed.forEach(([a, copies], i) => a && this.marks.set(a.id, this.owned(a, copies, 1_690_000_000 + i)));
  }

  private owned(a: Album, copies: OwnedCopy[], at: number): OwnedAlbum {
    const have = new Set(copies.map((c) => c.format));
    return { albumId: a.id, title: a.title, artist: a.artistTitle, thumb: a.thumb, copies, formats: FORMATS.filter((f) => have.has(f)), at };
  }

  list(): OwnedAlbum[] {
    return [...this.marks.values()].sort((a, b) => fold(a.artist).localeCompare(fold(b.artist)) || fold(a.title).localeCompare(fold(b.title)));
  }

  set(albumId: string, s: SetOwned): OwnedAlbum | null {
    if (!albumId.trim()) throw { kind: "invalid", message: "No album to mark." };
    if (s.copies.length > 20) throw { kind: "invalid", message: "An album can have up to 20 copies." };
    if (s.copies.length === 0) {
      this.marks.delete(albumId);
      this.emit();
      return null;
    }
    const prev = this.marks.get(albumId);
    const have = new Set(s.copies.map((c) => c.format));
    const o: OwnedAlbum = {
      albumId,
      title: s.title,
      artist: s.artist,
      thumb: s.thumb,
      copies: s.copies.map((c) => ({ ...c, pressing: c.pressing.trim(), colour: c.colour.trim(), condition: c.condition.trim(), notes: c.notes.trim() })),
      formats: FORMATS.filter((f) => have.has(f)),
      at: prev?.at ?? Math.floor(Date.now() / 1000),
    };
    this.marks.set(albumId, o);
    this.emit();
    return o;
  }

  stats(): ShelfStats {
    const by = new Map<PhysicalFormat, { albums: number; copies: number }>();
    let albums = 0;
    let copies = 0;
    const mostPlayed: ShelfAlbum[] = [];
    const lib = this.played();
    const shelf = (a: Album, formats: PhysicalFormat[]): ShelfAlbum => ({
      albumId: a.id,
      title: a.title,
      artist: a.artistTitle,
      thumb: a.thumb,
      plays: a.viewCount,
      formats,
    });
    for (const a of lib) {
      const o = this.marks.get(a.id);
      if (!o) continue;
      albums++;
      copies += o.copies.length;
      for (const f of o.formats) by.set(f, { albums: (by.get(f)?.albums ?? 0) + 1, copies: by.get(f)?.copies ?? 0 });
      for (const c of o.copies) by.set(c.format, { albums: by.get(c.format)?.albums ?? 0, copies: (by.get(c.format)?.copies ?? 0) + 1 });
      if (mostPlayed.length < 10 && a.viewCount > 0) mostPlayed.push(shelf(a, o.formats));
    }
    const missing = this.settings().shelfMissingList
      ? lib.filter((a) => a.viewCount > 0 && !this.marks.has(a.id)).slice(0, 25).map((a) => shelf(a, []))
      : null;
    return { albums, copies, byFormat: FORMATS.filter((f) => by.has(f)).map((f) => ({ format: f, ...by.get(f)! })), mostPlayed, missing };
  }

  // ---------------------------------------------------------------- Discogs

  status(): DiscogsStatus {
    return { saved: !!this.token, username: this.token ? this.username : null, importing: this.importing, report: this.token ? this.report : null };
  }

  private discogs() {
    if (this.fail === "network") throw { kind: "network", message: "Couldn't reach Discogs. Check your connection." };
    if (this.fail === "busy") throw { kind: "server", message: "Discogs is busy. Try again in a minute." };
  }

  setToken(token: string | null): DiscogsStatus {
    const t = token?.trim() ?? "";
    if (!t) {
      this.token = null;
      this.username = null;
      this.report = null;
      this.emit();
      return this.status();
    }
    this.checked.push(t);
    this.discogs();
    if (this.refuse) throw { kind: "invalid", message: "Discogs didn't accept that token. Copy a new one from your Discogs developer settings." };
    this.token = t;
    this.username = MOCK_DISCOGS_USER;
    this.emit();
    return this.status();
  }

  /** The faked collection: three records the library has, four it doesn't. */
  private collection(): FakeItem[] {
    const top = this.played();
    const item = (releaseId: number, a: Album | undefined, format: PhysicalFormat, pressing: string): FakeItem | null =>
      a ? { releaseId, artist: a.artistTitle, title: a.title, year: a.year, format, pressing, colour: "", condition: "" } : null;
    const owned = [item(9001, top[0], "vinyl", "LP, Album"), item(9002, top[1], "cd", "Album"), item(9003, top[3], "vinyl", "2xLP, Album, Gatefold")];
    const others: FakeItem[] = [
      { releaseId: 9101, artist: "Cold Coast", title: "Harbour Static", year: 2018, format: "vinyl", pressing: "LP, Album", colour: "Sea green", condition: "Mint" },
      { releaseId: 9102, artist: "Tidewater Hymns", title: "Saltmarsh", year: 2020, format: "cd", pressing: "Album", colour: "", condition: "" },
      { releaseId: 9103, artist: "Marrow & Vine", title: "Copper Seasons", year: 2019, format: "cassette", pressing: "Album, Limited Edition", colour: "Clear", condition: "" },
      { releaseId: 9104, artist: "The Quiet Ledger", title: "Basement Tapes Vol. 2", year: 1998, format: "other", pressing: "8-Track", colour: "", condition: "" },
    ];
    return [...owned.filter((x): x is FakeItem => !!x), ...others];
  }

  import(): DiscogsReport {
    if (!this.token) throw { kind: "invalid", message: "Add your Discogs token in Settings first." };
    if (this.importing) throw { kind: "invalid", message: "Your Discogs collection is already being read." };
    this.discogs();
    const items = this.collection();
    const byName = new Map(this.played().map((a) => [`${fold(a.artistTitle)}|${fold(a.title)}`, a]));
    const unmatched: ShelfOnly[] = [];
    let matched = 0;
    let added = 0;
    for (const it of items) {
      const a = byName.get(`${fold(it.artist)}|${fold(it.title)}`);
      if (!a) {
        unmatched.push({ releaseId: it.releaseId, artist: it.artist, title: it.title, year: it.year, format: it.format, thumb: null, links: links(it.artist, it.title) });
        continue;
      }
      matched++;
      const prev = this.marks.get(a.id);
      if (prev?.copies.some((c) => c.discogs?.releaseId === it.releaseId)) continue;
      if (!prev) added++;
      const c = copy(it.format, { pressing: it.pressing, colour: it.colour, condition: it.condition, discogs: { releaseId: it.releaseId, instanceId: it.releaseId * 10 } });
      this.marks.set(a.id, this.owned(a, [...(prev?.copies ?? []), c], prev?.at ?? Math.floor(Date.now() / 1000)));
    }
    unmatched.sort((x, y) => fold(x.artist).localeCompare(fold(y.artist)) || fold(x.title).localeCompare(fold(y.title)));
    this.report = { username: this.username ?? MOCK_DISCOGS_USER, importedAt: Math.floor(Date.now() / 1000), total: items.length, matched, added, unmatched };
    this.emit();
    return this.report;
  }
}
