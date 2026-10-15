/**
 * Mock sharing: Platter links built like the core's, invented public links
 * (no network), demos refused, and platter:// links opened through a test
 * hook (`openLink`) the way the OS hands them to the core.
 *
 * Some songs and albums have no public match, picked by id so the demo is
 * stable: those show the "Couldn't find" toast.
 */
import type { DeepLinkOpen, PublicLinkState, Settings, ShareInfo, ShareKind } from "../types";
import { type Fixtures, mockUnreleased } from "./fixtures";

const SERVER_TAG = "5e7c0a11";
const WORD: Record<ShareKind, string> = { track: "song", album: "album", artist: "artist", playlist: "playlist" };

function hash(s: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return h;
}

/**
 * A stand-in QR code for the demo: the three finder squares and a pattern
 * from the text's hash (not scannable; the core makes the real one).
 */
export function mockQr(text: string): { size: number; rows: string[] } {
  const size = 29;
  let h = hash(text);
  const finder = (x: number, y: number) => {
    for (const [fx, fy] of [[0, 0], [size - 7, 0], [0, size - 7]]) {
      const dx = x - fx;
      const dy = y - fy;
      if (dx >= 0 && dx < 7 && dy >= 0 && dy < 7) {
        const ring = Math.min(dx, dy, 6 - dx, 6 - dy);
        return ring !== 1;
      }
      if (dx >= -1 && dx <= 7 && dy >= -1 && dy <= 7) return false;
    }
    return null;
  };
  const rows: string[] = [];
  for (let y = 0; y < size; y++) {
    let r = "";
    for (let x = 0; x < size; x++) {
      const f = finder(x, y);
      if (f !== null) r += f ? "1" : "0";
      else {
        h = Math.imul(h ^ (x * 31 + y), 0x01000193) >>> 0;
        r += h % 7 < 3 ? "1" : "0";
      }
    }
    rows.push(r);
  }
  return { size, rows };
}

/** Ids with no confident match in the (pretend) store. */
export function mockHasNoMatch(id: string): boolean {
  return hash(id) % 5 === 0;
}

const fail = (kind: string, message: string) => ({ kind, message });

export class MockShare {
  private pending: DeepLinkOpen | null = null;
  /** Test hook: the next lookup fails like a dropped connection. */
  failNext = false;
  /** Lookups made (test hook). */
  lookups = 0;

  constructor(
    private fx: Fixtures,
    private settings: () => Settings,
    private playlistExists: (id: string) => boolean,
    private waiting: () => void,
  ) {}

  private album(id: string) {
    return this.fx.albums.find((a) => a.id === id);
  }

  private exists(kind: ShareKind, id: string): boolean {
    switch (kind) {
      case "track":
        return this.fx.tracks.some((t) => t.id === id);
      case "album":
        return !!this.album(id);
      case "artist":
        return this.fx.artists.some((a) => a.id === id);
      case "playlist":
        return this.playlistExists(id);
    }
  }

  private unreleased(kind: ShareKind, id: string): boolean {
    const albumId = kind === "track" ? this.fx.tracks.find((t) => t.id === id)?.albumId : id;
    const a = albumId ? this.album(albumId) : undefined;
    return !!a && mockUnreleased(a, this.settings().countUnmatchedUnreleased !== false);
  }

  private status(kind: ShareKind, id: string): PublicLinkState {
    if (kind === "artist" || kind === "playlist") return "none";
    if (this.unreleased(kind, id)) return "unreleased";
    return this.settings().sharePublicLinks ? "available" : "off";
  }

  info(kind: ShareKind, id: string, atSeconds?: number): ShareInfo {
    if (!this.exists(kind, id)) throw fail("notFound", "This isn't in your library");
    const q = [kind === "track" && atSeconds != null ? `t=${Math.max(0, Math.floor(atSeconds))}` : null, `s=${SERVER_TAG}`].filter(Boolean);
    return { appLink: `platter://${WORD[kind]}/${encodeURIComponent(id)}?${q.join("&")}`, public: this.status(kind, id) };
  }

  publicLink(kind: ShareKind, id: string): string {
    if (!this.exists(kind, id)) throw fail("notFound", "This isn't in your library");
    const st = this.status(kind, id);
    if (st === "unreleased") throw fail("invalid", "Not released");
    if (st === "off") throw fail("invalid", "Public links are off in Settings");
    if (st === "none") throw fail("invalid", "Only songs and albums get public links");
    this.lookups++;
    if (this.failNext) {
      this.failNext = false;
      throw fail("network", "Couldn't reach the link search. Try again in a minute.");
    }
    if (mockHasNoMatch(id)) throw fail("notFound", kind === "album" ? "Couldn't find this album online" : "Couldn't find this song online");
    const n = 1_000_000_000 + (hash(`${kind}:${id}`) % 900_000_000);
    return kind === "album" ? `https://album.link/i/${n}` : `https://song.link/i/${n}`;
  }

  /** Read a platter:// link like the core does (enough for the demo). */
  resolve(raw: string): DeepLinkOpen {
    const none = (status: DeepLinkOpen["status"]): DeepLinkOpen => ({ status, kind: null, id: null, albumId: null, atSeconds: null });
    if (/^platter:\/\/join\/[0-9a-f]{16}\?/i.test(raw.trim())) return { ...none("open"), join: raw.trim() };
    const m =/^platter:\/\/(song|track|album|artist|playlist)\/([A-Za-z0-9_-]{1,64})\/?(?:\?(.*))?$/i.exec(raw.trim());
    if (!m) return none("invalid");
    const kind: ShareKind = m[1].toLowerCase() === "song" || m[1].toLowerCase() === "track" ? "track" : (m[1].toLowerCase() as ShareKind);
    const id = m[2];
    const params = new URLSearchParams(m[3] ?? "");
    const s = params.get("s");
    if (s && s !== SERVER_TAG) return none("notInLibrary");
    if (!this.exists(kind, id)) return none("notInLibrary");
    const t = params.get("t");
    const atSeconds = kind === "track" && t && /^\d+$/.test(t) ? Number(t) : null;
    const albumId = kind === "track" ? (this.fx.tracks.find((x) => x.id === id)?.albumId ?? null) : null;
    return { status: "open", kind, id, albumId, atSeconds };
  }

  /** Test/demo hook: the OS opened a platter:// link. */
  openLink(raw: string) {
    this.pending = this.resolve(raw);
    this.waiting();
  }

  take(): DeepLinkOpen | null {
    const p = this.pending;
    this.pending = null;
    return p;
  }
}
