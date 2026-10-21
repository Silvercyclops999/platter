/**
 * Mock friend activity, follows, What's New and listening sessions.
 * Invented people and fixture music only.
 *
 * Test hooks: `access` ("ok", "notAdmin", "offline"), `failNextJoin`,
 * `guestJoins(name)`, `hostGone()`, `hostEnds()`, `removedByHost()`,
 * `relay(health)`, `setHostControl(on)`, `addRelease(albumId)`, `songNotHere(title)`,
 * `anotherVersion(on)`, `guestCantHear(name, on)`, `guestTakesInvites(on)`,
 * `guestAccepts(name)`, `hostCopyHere()`.
 *
 * Invites look like the real ones: a random token and nothing else.
 */
import type { ActivityPanel, Album, FollowedArtist, FriendActivity, ListenInvite, ListenOffer, ListenOp, ListenView, ShareStatus, Track, WhatsNew } from "../types";
import type { Fixtures } from "./fixtures";
import { mockQr } from "./sync";

/** Invented people on the mock server. Avatars are left to the letter fallback. */
export const MOCK_FRIENDS = ["Robin", "Jules", "Morgan", "Casey"];

const MIN = 60;
const HOUR = 3600;
const DAY = 86400;

export interface SocialHooks {
  settings: () => { listenGuestsControl: boolean; listenInviteHours: number; listenName: string; showFriendActivity: boolean; shareOfferServer: boolean };
  emitFollows: (all: FollowedArtist[]) => void;
  emitBadge: (unseen: number) => void;
  emitListen: (v: ListenView) => void;
  later: (ms: number, fn: () => void) => void;
  /** The browser demo (its timelines run): someone opens a new session's invite by themselves. Tests join with guestJoins. */
  demo: () => boolean;
  queueAdd: (ids: string[]) => void;
  /** Play, pause, skip or seek on the mock player (a guest's request reaching the host). */
  control: (op: ListenOp) => void;
  /** Share the host's music with a guest's Plex account (the default libraries). */
  shareWithGuest: (plexUser: string) => ShareStatus;
  /** The host told this guest it shared its server: an invite arrives. */
  hostShared: (host: string) => void;
}

const NONE: ListenView = {
  role: "none",
  sessionId: null,
  hostName: null,
  invite: null,
  people: [],
  listeners: [],
  guestsCanControl: false,
  nowBy: null,
  queue: [],
  status: null,
  relay: "ok",
  cantHear: [],
  missing: null,
  anotherVersion: false,
  offerServer: false,
};
export const MOCK_INVITE = "platter://join/AV8MKp57HUw4puL5CxfT5EXMOOHwS3stlqDkyLHT9XopaPBRgA";

export class MockSocial {
  access: "ok" | "notAdmin" | "offline" = "ok";
  /** Rows after the demo friends (a test hook: plays the core couldn't link). */
  extraFriends: FriendActivity[] = [];
  failNextJoin: string | null = null;
  private follows: FollowedArtist[] = [];
  /** Albums already seen on What's New. */
  private seen = new Set<string>();
  private extra: string[] = [];
  private listen: ListenView = { ...NONE };
  private invites = 0;
  private hostControl = true;
  /** Guests' Platters answer with their Plex name (off: "Accept invites inside Platter" is off there). */
  private guestsTakeInvites = true;

  constructor(
    private fx: Fixtures,
    private hooks: SocialHooks,
  ) {
    this.reset();
  }

  reset() {
    const now = Math.floor(Date.now() / 1000);
    this.access = "ok";
    this.extraFriends = [];
    this.failNextJoin = null;
    this.extra = [];
    this.follows = [this.fx.artists[0], this.fx.artists[2]].map((a, i) => ({ id: a.id, title: a.title, thumb: a.thumb, followedAt: now - (i + 3) * DAY }));
    // The two newest releases start unseen.
    const all = this.releases();
    this.seen = new Set(all.slice(2).map((a) => a.id));
    this.listen = { ...NONE };
    this.invites = 0;
    this.hostControl = true;
    this.guestsTakeInvites = true;
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
        artistId: t.artistId,
        albumId: t.albumId,
        album: t.albumTitle,
        cover: t.thumb,
        at: now - ago[i],
        now: i === 0,
        inLibrary: true,
      };
    });
    return { state: "ok", friends: [...friends, ...this.extraFriends] };
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

  justShow(id: string, justShow: boolean): FollowedArtist[] {
    const f = this.follows.find((x) => x.id === id);
    if (!f) throw { kind: "invalid", message: "Follow the artist first." };
    if (!!f.justShow === justShow) return this.list();
    if (justShow) f.justShow = true;
    else delete f.justShow;
    this.hooks.emitFollows(this.list());
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
    return {
      ...this.listen,
      people: this.listen.people.map((p) => ({ ...p, offer: p.offer ? { ...p.offer } : null })),
      listeners: [...this.listen.listeners],
      queue: [...this.listen.queue],
      cantHear: [...this.listen.cantHear],
      missing: this.listen.missing && { ...this.listen.missing },
    };
  }

  private setListen(v: ListenView) {
    v.listeners = v.people.filter((p) => !p.me).map((p) => p.name);
    v.cantHear = v.role === "host" ? v.people.filter((p) => p.cantHear).map((p) => p.name) : [];
    this.listen = v;
    this.hooks.emitListen(this.listenState());
  }

  private newInvite(): ListenInvite {
    this.invites += 1;
    const link = this.invites === 1 ? MOCK_INVITE : `${MOCK_INVITE.slice(0, -2)}${String(this.invites).padStart(2, "0")}`;
    return { link, qr: mockQr(link), expiresAt: Date.now() + this.hooks.settings().listenInviteHours * HOUR * 1000 };
  }

  start(): ListenView {
    if (this.listen.role === "guest") throw { kind: "invalid", message: "Leave the listening session you're in first." };
    if (this.listen.role === "none") {
      const name = this.hooks.settings().listenName || "listener";
      this.setListen({
        ...NONE,
        role: "host",
        sessionId: "5f0c2a9e7b1d4c38a6e2f90b1c7d3e45",
        hostName: name,
        invite: this.newInvite(),
        people: [{ id: "host", name, avatar: null, host: true, me: true, cantHear: false }],
        guestsCanControl: this.hooks.settings().listenGuestsControl,
        offerServer: this.hooks.settings().shareOfferServer,
        status: "live",
        relay: "ok",
      });
      // In the demo someone opens the link a moment later.
      if (this.hooks.demo()) this.hooks.later(1500, () => this.guestJoins(MOCK_FRIENDS[0]));
    }
    return this.listenState();
  }

  invite(fresh: boolean): ListenView {
    if (this.listen.role !== "host") throw { kind: "invalid", message: "Start a listening session first." };
    if (fresh || !this.listen.invite) this.setListen({ ...this.listen, invite: this.newInvite() });
    return this.listenState();
  }

  /** A guest joins the session this computer hosts (test hook); the invite is used up. */
  guestJoins(name: string) {
    if (this.listen.role !== "host" || this.listen.people.some((p) => p.name === name)) return;
    const id = `g${this.listen.people.length}0c4e1a7b9d2f3`;
    this.setListen({ ...this.listen, invite: null, people: [...this.listen.people, { id, name, avatar: null, host: false, me: false, cantHear: false }] });
  }

  remove(personId: string): ListenView {
    if (this.listen.role !== "host") throw { kind: "invalid", message: "Only the host can remove people." };
    if (!this.listen.people.some((p) => p.id === personId && !p.host)) throw { kind: "notFound", message: "They're not in the session anymore." };
    this.setListen({ ...this.listen, people: this.listen.people.filter((p) => p.id !== personId) });
    return this.listenState();
  }

  stop() {
    if (this.listen.role !== "none") this.setListen({ ...NONE, people: [] });
  }

  join(link: string): ListenView {
    if (!link.startsWith("platter://join/")) throw { kind: "invalid", message: "That isn't a listening session link." };
    if (this.listen.role === "host") throw { kind: "invalid", message: "End your own listening session first." };
    if (this.failNextJoin) {
      const message = this.failNextJoin;
      this.failNextJoin = null;
      throw { kind: "invalid", message };
    }
    const host = MOCK_FRIENDS[1];
    const other = { id: "a91f3c0d5e7b2468", name: MOCK_FRIENDS[2], avatar: null, host: false, me: false, cantHear: false };
    const t = this.fx.tracks;
    this.setListen({
      ...NONE,
      role: "guest",
      sessionId: "c3a8e1f04b7d2956a0e4c8b1d3f57a29",
      hostName: host,
      people: [
        { id: "host", name: host, avatar: null, host: true, me: false, cantHear: false },
        other,
        { id: "me", name: this.hooks.settings().listenName || "listener", avatar: null, host: false, me: true, cantHear: false },
      ],
      guestsCanControl: this.hostControl,
      nowBy: other.id,
      queue: [
        { trackId: t[11].id, by: null },
        { trackId: t[12].id, by: other.id },
      ],
      status: "live",
      relay: "ok",
    });
    return this.listenState();
  }

  control(op: ListenOp) {
    const v = this.listen;
    if (v.role === "none" || (v.role === "guest" && v.status !== "live")) throw { kind: "invalid", message: "You're not in a listening session." };
    if (v.role === "guest" && !v.guestsCanControl && op.op !== "add") throw { kind: "invalid", message: `${v.hostName} only lets others add songs.` };
    if (op.op === "add") {
      this.hooks.queueAdd(op.ids);
      if (v.role === "guest") this.setListen({ ...v, queue: [...v.queue, ...op.ids.map((trackId) => ({ trackId, by: "me" }))] });
      return;
    }
    this.hooks.control(op);
  }

  add(ids: string[]) {
    this.control({ op: "add", ids });
  }

  /** The host turns "Let others change what's playing" on or off (test hook). */
  setHostControl(on: boolean) {
    this.hostControl = on;
    if (this.listen.role === "guest") this.setListen({ ...this.listen, guestsCanControl: on });
  }

  /** The host's own setting changed. */
  settingsChanged() {
    const s = this.hooks.settings();
    if (this.listen.role === "host") this.setListen({ ...this.listen, guestsCanControl: s.listenGuestsControl, offerServer: s.shareOfferServer });
  }

  private setOffer(personId: string, offer: ListenOffer | null) {
    if (this.listen.role !== "host") return;
    this.setListen({ ...this.listen, people: this.listen.people.map((p) => (p.id === personId ? { ...p, offer } : p)) });
  }

  /** "Let them listen from my server": ask the guest's Platter, then share through Plex. */
  offerServer(personId: string): ListenView {
    if (!this.hooks.settings().shareOfferServer) throw { kind: "invalid", message: "Sharing your server is turned off in Settings, Sharing." };
    if (this.listen.role !== "host") throw { kind: "invalid", message: "Only the host can share their server." };
    const p = this.listen.people.find((x) => x.id === personId && !x.host);
    if (!p) throw { kind: "notFound", message: "They're not in the session anymore." };
    this.setOffer(personId, { state: "asking" });
    this.hooks.later(400, () => {
      if (!this.guestsTakeInvites) return this.setOffer(personId, { state: "noInvites" });
      this.setOffer(personId, { state: "inviting" });
      this.hooks.later(500, () => {
        const st = this.hooks.shareWithGuest(`${p.name.toLowerCase()}_on_plex`);
        const offer: ListenOffer =
          st.status === "invited"
            ? { state: "invited" }
            : st.status === "alreadyShared"
              ? { state: "alreadyShared" }
              : { state: "failed", message: "message" in st ? st.message : "Your unreleased music can only be kept out with Plex Pass. Share from Settings, Sharing to choose what to do." };
        this.setOffer(personId, offer);
        // In the demo they accept a few seconds later and hear your copy.
        if (offer.state === "invited" && this.hooks.demo()) this.hooks.later(3000, () => this.guestAccepts(p.name));
      });
    });
    return this.listenState();
  }

  /** Whether guests' Platters take invites inside Platter (test hook). */
  guestTakesInvites(on: boolean) {
    this.guestsTakeInvites = on;
  }

  /** A guest accepted and plays the host's copy now (test hook). */
  guestAccepts(name: string) {
    this.guestCantHear(name, false);
  }

  /** The host shared their server with this guest (test hook): the invite shows here. */
  hostSharesServer() {
    if (this.listen.role === "guest" && this.listen.hostName) this.hooks.hostShared(this.listen.hostName);
  }

  /** The host's server is in the library now: its copy of the song plays. */
  hostCopyHere() {
    if (this.listen.role === "guest") this.setListen({ ...this.listen, missing: null, anotherVersion: false });
  }

  /** The host stops answering (test hook). */
  hostGone() {
    if (this.listen.role === "guest") this.setListen({ ...this.listen, status: "hostGone" });
  }

  /** The host ends the session (test hook). */
  hostEnds() {
    if (this.listen.role === "guest") this.setListen({ ...this.listen, status: "ended" });
  }

  /** The host removes this guest (test hook). */
  removedByHost() {
    if (this.listen.role === "guest") this.setListen({ ...this.listen, status: "removed" });
  }

  /** The host's song has no copy here (`null`: back to one that has), test hook. */
  songNotHere(title: string | null, unreleased = false) {
    if (this.listen.role !== "guest") return;
    const missing = title === null && !unreleased ? null : { title, artist: title === null ? null : "Fixture Artist", unreleased };
    this.setListen({ ...this.listen, missing, anotherVersion: false });
  }

  /** This guest plays a copy of another length (test hook). */
  anotherVersion(on: boolean) {
    if (this.listen.role === "guest") this.setListen({ ...this.listen, missing: null, anotherVersion: on });
  }

  /** A guest says they can't hear the song playing, or can again (test hook). */
  guestCantHear(name: string, on: boolean) {
    if (this.listen.role !== "host") return;
    this.setListen({ ...this.listen, people: this.listen.people.map((p) => (p.name === name ? { ...p, cantHear: on } : p)) });
  }

  /** The relay can't be reached, is busy, or is back (test hook). */
  relay(health: ListenView["relay"]) {
    if (this.listen.role !== "none") this.setListen({ ...this.listen, relay: health });
  }
}
