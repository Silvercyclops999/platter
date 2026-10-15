/**
 * Mock friend activity, follows, What's New and listening sessions.
 * Invented people and fixture music only.
 *
 * Test hooks: `access` ("ok", "notAdmin", "offline"), `failNextJoin`,
 * `guestJoins(name)`, `hostGone()`, `hostEnds()`, `addRelease(albumId)`.
 */
import type { ActivityPanel, Album, FollowedArtist, FriendActivity, ListenView, Track, WhatsNew } from "../types";
import type { Fixtures } from "./fixtures";

/** Invented people on the mock server. Avatars are left to the letter fallback. */
export const MOCK_FRIENDS = ["Robin", "Jules", "Morgan", "Casey"];

const MIN = 60;
const HOUR = 3600;
const DAY = 86400;

export interface SocialHooks {
  settings: () => { listenGuestsAdd: boolean; showFriendActivity: boolean };
  emitFollows: (all: FollowedArtist[]) => void;
  emitBadge: (unseen: number) => void;
  emitListen: (v: ListenView) => void;
  later: (ms: number, fn: () => void) => void;
  queueAdd: (ids: string[]) => void;
}

const NONE: ListenView = { role: "none", sessionId: null, hostName: null, invite: null, listeners: [], guestsCanAdd: false, status: null };
export const MOCK_INVITE = "platter://join/0123456789abcdef?k=00112233445566778899aabbccddeeff&s=0a1b2c3d&h=10.0.0.5%3A32520&n=listener";

export class MockSocial {
  access: "ok" | "notAdmin" | "offline" = "ok";
  failNextJoin: string | null = null;
  private follows: FollowedArtist[] = [];
  /** Albums already seen on What's New. */
  private seen = new Set<string>();
  private extra: string[] = [];
  private listen: ListenView = { ...NONE };

  constructor(
    private fx: Fixtures,
    private hooks: SocialHooks,
  ) {
    this.reset();
  }

  reset() {
    const now = Math.floor(Date.now() / 1000);
    this.access = "ok";
    this.failNextJoin = null;
    this.extra = [];
    this.follows = [this.fx.artists[0], this.fx.artists[2]].map((a, i) => ({ id: a.id, title: a.title, thumb: a.thumb, followedAt: now - (i + 3) * DAY }));
    // The two newest releases start unseen.
    const all = this.releases();
    this.seen = new Set(all.slice(2).map((a) => a.id));
    this.listen = { ...NONE };
  }

  // ---------------------------------------------------------------- activity

  activity(): ActivityPanel {
    if (!this.hooks.settings().showFriendActivity) return { state: "off", friends: [] };
    if (this.access !== "ok") return { state: this.access, friends: [] };
    const now = Math.floor(Date.now() / 1000);
    const ago = [0, 11 * MIN, 2 * HOUR, DAY + 3 * HOUR];
    const friends: FriendActivity[] = MOCK_FRIENDS.map((name, i) => {
      const t: Track = this.fx.tracks[(i * 37 + 5) % this.fx.tracks.length];
      return {
        accountId: i + 2,
        name,
        thumb: null,
        trackId: t.id,
        title: t.title,
        artist: t.trackArtist ?? t.artistTitle,
        albumId: t.albumId,
        album: t.albumTitle,
        cover: t.thumb,
        at: now - ago[i],
        now: i === 0,
      };
    });
    return { state: "ok", friends };
  }

  // ---------------------------------------------------------------- follows

  list(): FollowedArtist[] {
    return this.follows.map((f) => ({ ...f }));
  }

  set(artist: { id: string; title: string; thumb?: string | null }, on: boolean): FollowedArtist[] {
    if (!artist.id.trim()) throw { kind: "invalid", message: "Not an artist." };
    const had = this.follows.some((f) => f.id === artist.id);
    if (on && !had) {
      this.follows.push({ id: artist.id, title: artist.title, thumb: artist.thumb ?? null, followedAt: Math.floor(Date.now() / 1000) });
      // What they already had isn't new.
      for (const a of this.albumsOf(artist.id)) this.seen.add(a.id);
    } else if (!on && had) {
      this.follows = this.follows.filter((f) => f.id !== artist.id);
    } else return this.list();
    this.hooks.emitFollows(this.list());
    this.hooks.emitBadge(this.whatsNew().unseen);
    return this.list();
  }

  private albumsOf(artistId: string): Album[] {
    return this.fx.albums.filter((a) => a.artistId === artistId);
  }

  private releases(): Album[] {
    const ids = new Set(this.follows.map((f) => f.id));
    const pool = ids.size ? this.fx.albums.filter((a) => ids.has(a.artistId)) : this.fx.albums;
    const extra = this.extra.map((id) => this.fx.albums.find((a) => a.id === id)).filter((a): a is Album => !!a);
    const byNew = [...pool].sort((a, b) => b.addedAt - a.addedAt);
    const seen = new Set<string>();
    return [...extra, ...byNew].filter((a) => !seen.has(a.id) && !!seen.add(a.id)).slice(0, 20);
  }

  whatsNew(): WhatsNew {
    const items = this.releases().map((album) => ({ album: { ...album }, new: !this.seen.has(album.id) }));
    return { following: this.follows.length > 0, items, unseen: items.filter((i) => i.new).length };
  }

  markSeen() {
    for (const a of this.releases()) this.seen.add(a.id);
    this.hooks.emitBadge(0);
  }

  /** A followed artist's album lands in the library (test hook). */
  addRelease(albumId: string) {
    this.extra.unshift(albumId);
    this.seen.delete(albumId);
    this.hooks.emitBadge(this.whatsNew().unseen);
  }

  // ---------------------------------------------------------------- listen along

  listenState(): ListenView {
    return { ...this.listen, listeners: [...this.listen.listeners] };
  }

  private setListen(v: ListenView) {
    this.listen = v;
    this.hooks.emitListen(this.listenState());
  }

  start(): ListenView {
    if (this.listen.role === "guest") throw { kind: "invalid", message: "Leave the listening session you're in first." };
    if (this.listen.role === "none") {
      this.setListen({
        role: "host",
        sessionId: "0123456789abcdef",
        hostName: "listener",
        invite: MOCK_INVITE,
        listeners: [],
        guestsCanAdd: this.hooks.settings().listenGuestsAdd,
        status: "live",
      });
      // Someone opens the link a moment later.
      this.hooks.later(1500, () => this.guestJoins(MOCK_FRIENDS[0]));
    }
    return this.listenState();
  }

  /** A guest joins the session this computer hosts (test hook). */
  guestJoins(name: string) {
    if (this.listen.role !== "host" || this.listen.listeners.includes(name)) return;
    this.setListen({ ...this.listen, listeners: [...this.listen.listeners, name] });
  }

  stop() {
    if (this.listen.role !== "none") this.setListen({ ...NONE });
  }

  join(link: string): ListenView {
    if (!link.startsWith("platter://join/")) throw { kind: "invalid", message: "That isn't a listening session link." };
    if (this.listen.role === "host") throw { kind: "invalid", message: "End your own listening session first." };
    if (this.failNextJoin) {
      const message = this.failNextJoin;
      this.failNextJoin = null;
      throw { kind: "network", message };
    }
    this.setListen({
      role: "guest",
      sessionId: "fedcba9876543210",
      hostName: MOCK_FRIENDS[1],
      invite: null,
      listeners: [MOCK_FRIENDS[1], MOCK_FRIENDS[2]],
      guestsCanAdd: this.hooks.settings().listenGuestsAdd,
      status: "live",
    });
    return this.listenState();
  }

  add(ids: string[]) {
    if (this.listen.role === "none") throw { kind: "invalid", message: "You're not in a listening session." };
    if (this.listen.role === "guest" && !this.listen.guestsCanAdd) throw { kind: "invalid", message: "The host doesn't let guests add songs." };
    this.hooks.queueAdd(ids);
  }

  /** The host stops answering (test hook). */
  hostGone() {
    if (this.listen.role === "guest") this.setListen({ ...this.listen, status: "hostGone" });
  }

  /** The host ends the session (test hook). */
  hostEnds() {
    if (this.listen.role === "guest") this.setListen({ ...this.listen, status: "ended" });
  }
}
