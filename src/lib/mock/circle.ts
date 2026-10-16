/**
 * Mock shared playlists and Blends, with invented people and fixture music
 * only. Members and the relay are simulated: people join a moment after an
 * invite goes out, a collaborator adds a song now and then (browser demo),
 * and edits you make on someone else's playlist travel to its owner and
 * come back confirmed.
 *
 * Test hooks: `relay(health)`, `ownerOffline(on)`, `memberAdds(name,
 * trackId)`, `ownerDeletes()`, `failNextJoin("expired" | "used" | "relay")`,
 * `joinWaits(on)`, `ring(from, kind, title)`, `personJoins(playlistId, name,
 * role)`, `blendJoins(id, name)`.
 *
 * Invite links look like the real ones: a random token and nothing else.
 */
import type {
  BlendCreated,
  BlendMissing,
  BlendPage,
  BlendSummary,
  CircleAccount,
  CircleIncoming,
  CirclePerson,
  CircleView,
  CollabRole,
  InviteLink,
  JoinResult,
  Playlist,
  SharedInfo,
  SharedPage,
  SharedRow,
  SharedSummary,
  Track,
} from "../types";
import type { Fixtures } from "./fixtures";

const avatar = (n: number) => `/ext/https://plex.tv/users/mock${n}/avatar?c=1`;

/** A token like the core's: 37+ random-looking bytes, base64url. */
function token(seed: number): string {
  const chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
  let x = seed * 2654435761 + 12345;
  let s = "AQ";
  for (let i = 0; i < 48; i++) {
    x = (x * 1103515245 + 12345) & 0x7fffffff;
    s += chars[x % 64];
  }
  return s;
}

export const MOCK_COLLAB_INVITE = `platter://collab/${token(1)}`;
export const MOCK_BLEND_INVITE = `platter://blend/${token(2)}`;
/** The playlist shared with you in the demo. */
export const MOCK_SHARED_ID = "shared:5f1d2c3b4a5968778695a4b3c2d1e0f9";
/** The demo Blend. */
export const MOCK_BLEND_ID = "blend:0a1b2c3d4e5f60718293a4b5c6d7e8f9";
/** Your own playlist that's already shared in the demo. */
export const MOCK_OWNED_SHARED = "502";

export interface CircleHooks {
  settings: () => { collabPlaylists: boolean; blends: boolean; shareUnreleased: boolean };
  me: () => { name: string; avatar: string | null };
  /** Other people on the server and in the Plex Home. */
  people: () => CircleAccount[];
  emit: (v: CircleView) => void;
  notice: (message: string) => void;
  later: (ms: number, fn: () => void) => void;
  /** The browser demo: people join and add songs by themselves. */
  demo: () => boolean;
  track: (id: string) => Track | undefined;
  tracks: () => Track[];
  playlist: (id: string) => Playlist | undefined;
  itemIds: (id: string) => string[];
  trackIds: (id: string) => string[];
  /** Add to one of your playlists (a collaborator's song, applied by you). */
  add: (playlistId: string, trackIds: string[]) => void;
}

interface Owned {
  playlistId: string;
  group: string;
  public: boolean;
  members: { p: CirclePerson; role: CollabRole }[];
  /** item id → person id. */
  addedBy: Map<string, string>;
}

interface Joined {
  id: string;
  title: string;
  description: string | null;
  owner: CirclePerson;
  role: CollabRole;
  public: boolean;
  people: CirclePerson[];
  rows: SharedRow[];
  gone: boolean;
  inLibrary: boolean;
  pending: SharedRow[];
  removing: Set<string>;
}

interface Blend {
  id: string;
  others: CirclePerson[];
  hidden: Set<string>;
  day: number;
}

const err = (kind: string, message: string) => ({ kind, message });

let seq = 10;
const rid = () => `p${seq++}`;

function person(name: string, n: number | null, role: CollabRole | null = null, you = false): CirclePerson {
  return { id: rid(), name, avatar: n === null ? null : avatar(n), role, you, added: 0 };
}

export class MockCircle {
  private owned: Owned[] = [];
  private joined: Joined[] = [];
  private blends: Blend[] = [];
  private incoming: CircleIncoming[] = [];
  private health: CircleView["relay"] = "ok";
  private offline = false;
  private tries = 0;
  private waits = false;
  private nextFail: "expired" | "used" | "relay" | null = null;
  private joining = 0;
  private invites = 0;
  private youId = "you";

  constructor(
    private fx: Fixtures,
    private h: CircleHooks,
  ) {
    this.reset();
  }

  reset() {
    this.health = "ok";
    this.offline = false;
    this.tries = 0;
    this.waits = false;
    this.nextFail = null;
    this.joining = 0;
    this.incoming = [];
    this.youId = "you";
    const tracks = this.fx.tracks;
    // Your "Workout Rotation" is shared with Robin (collaborator, added two songs) and Jules.
    const robin = person("Robin", 1, "collaborator");
    const jules = person("Jules", 2, "listener");
    const items = this.h.itemIds(MOCK_OWNED_SHARED);
    const addedBy = new Map<string, string>();
    if (items.length > 3) {
      addedBy.set(items[1], robin.id);
      addedBy.set(items[3], robin.id);
    }
    this.owned = [{ playlistId: MOCK_OWNED_SHARED, group: "c0ffee00c0ffee00c0ffee00c0ffee00", public: false, members: [{ p: robin, role: "collaborator" }, { p: jules, role: "listener" }], addedBy }];
    // Morgan's "Road Trip" is shared with you as a collaborator.
    const morgan = person("Morgan", 3, "owner");
    const casey = person("Casey", null, "listener");
    const you = { ...person(this.h.me().name, 0, "collaborator", true), id: this.youId };
    const picks = tracks.filter((_, i) => i % 37 === 5).slice(0, 9);
    const rows: SharedRow[] = picks.map((t, i) => this.row(t, i % 3 === 1 ? you : i % 3 === 2 ? casey : morgan, i));
    // Two songs Morgan has that you don't.
    rows.splice(3, 0, this.missingRow("Harbour Lights", "The Night Ferries", "Coastline", morgan, 30));
    rows.push(this.missingRow("Paper Planes Again", "Velvet Static", "Fold Lines", casey, 31));
    this.joined = [
      {
        id: MOCK_SHARED_ID,
        title: "Road Trip",
        description: "Songs for the long way round.",
        owner: morgan,
        role: "collaborator",
        public: false,
        people: [morgan, you, casey],
        rows,
        gone: false,
        inLibrary: true,
        pending: [],
        removing: new Set(),
      },
    ];
    this.blends = [{ id: MOCK_BLEND_ID, others: [person("Robin", 1), person("Jules", 2)], hidden: new Set(), day: 0 }];
    this.changed();
  }

  private row(t: Track, by: CirclePerson | null, i: number): SharedRow {
    return {
      key: `k${t.id}-${i}`,
      track: t,
      title: t.title,
      artist: t.trackArtist ?? t.artistTitle,
      album: t.albumTitle,
      durationMs: t.durationMs,
      releaseGroupId: null,
      by: by && by.role !== "owner" ? by : null,
      addedAt: Math.floor(Date.now() / 1000) - (20 - i) * 86400,
      pending: false,
      removable: !!by?.you,
    };
  }

  private missingRow(title: string, artist: string, album: string, by: CirclePerson, i: number): SharedRow {
    return {
      key: `missing-${i}`,
      track: null,
      title,
      artist,
      album,
      durationMs: 214_000,
      releaseGroupId: `00000000-0000-4000-8000-0000000000${i}`,
      by: by.role === "owner" ? null : by,
      addedAt: Math.floor(Date.now() / 1000) - 5 * 86400,
      pending: false,
      removable: false,
    };
  }

  // ---------------------------------------------------------------- views

  private ownedSummary(o: Owned): SharedSummary {
    const p = this.h.playlist(o.playlistId);
    const me = this.h.me();
    const counts = new Map<string, number>();
    for (const [item, by] of o.addedBy) if (this.h.itemIds(o.playlistId).includes(item)) counts.set(by, (counts.get(by) ?? 0) + 1);
    const owner: CirclePerson = { id: this.youId, name: me.name, avatar: me.avatar, role: "owner", you: true, added: 0 };
    return {
      id: o.playlistId,
      group: o.group,
      title: p?.title ?? "Playlist",
      owner,
      role: "owner",
      people: [owner, ...o.members.filter((m) => m.role !== "follower").map((m) => ({ ...m.p, role: m.role, added: counts.get(m.p.id) ?? 0 }))],
      public: o.public,
      gone: !p,
      inLibrary: true,
      waiting: 0,
      ownerUnreachable: false,
      trackCount: p?.trackCount ?? 0,
    };
  }

  private joinedSummary(j: Joined): SharedSummary {
    const counts = new Map<string, number>();
    for (const r of j.rows) if (r.by) counts.set(r.by.id, (counts.get(r.by.id) ?? 0) + 1);
    return {
      id: j.id,
      group: j.id.slice(7),
      title: j.title,
      owner: j.owner,
      role: j.role,
      people: j.people.filter((p) => p.role !== "follower").map((p) => ({ ...p, added: counts.get(p.id) ?? 0 })),
      public: j.public,
      gone: j.gone,
      inLibrary: j.inLibrary,
      waiting: j.pending.length + j.removing.size,
      ownerUnreachable: this.offline && this.tries >= 3,
      trackCount: j.rows.length,
    };
  }

  private blendNames(b: Blend): string[] {
    return [this.h.me().name, ...b.others.map((o) => o.name)];
  }

  private blendSummary(b: Blend): BlendSummary {
    const names = this.blendNames(b);
    const list = names.length === 2 ? `${names[0]} and ${names[1]}` : `${names.slice(0, -1).join(", ")}, and ${names[names.length - 1]}`;
    return {
      id: b.id,
      title: names.join(" + "),
      description: names.length < 2 ? `A Blend of music for ${names[0]}` : `A Blend of music for ${list}`,
      people: [this.meInBlend(), ...b.others],
      trackCount: this.blendRows(b).length,
    };
  }

  view(): CircleView {
    return {
      relay: this.health,
      playlists: [...this.owned.filter((o) => o.members.length).map((o) => this.ownedSummary(o)), ...this.joined.map((j) => this.joinedSummary(j))],
      blends: this.blends.map((b) => this.blendSummary(b)),
      incoming: this.incoming.map((i) => ({ ...i })),
      joining: this.joining,
    };
  }

  private changed() {
    this.h.emit(this.view());
  }

  // ---------------------------------------------------------------- commands

  private need(kind: "playlist" | "blend") {
    const s = this.h.settings();
    if (kind === "playlist" && !s.collabPlaylists) throw err("invalid", "Collaborative playlists are off in Settings.");
    if (kind === "blend" && !s.blends) throw err("invalid", "Blends are off in Settings.");
  }

  people(): CircleAccount[] {
    return this.h.people();
  }

  private link(kind: "collab" | "blend"): InviteLink {
    if (this.health === "unreachable") throw err("network", "Couldn't reach the relay. Changes wait here and go out when it's back.");
    return { link: `platter://${kind}/${token(100 + ++this.invites)}`, expiresAt: Date.now() + 24 * 3600_000 };
  }

  private ownedFor(playlistId: string): Owned {
    let o = this.owned.find((x) => x.playlistId === playlistId);
    if (!o) {
      if (!this.h.playlist(playlistId)) throw err("notFound", "This playlist was deleted.");
      o = { playlistId, group: `${playlistId}`.padStart(32, "0"), public: false, members: [], addedBy: new Map() };
      this.owned.push(o);
    }
    return o;
  }

  info(playlistId: string): SharedInfo | null {
    const o = this.owned.find((x) => x.playlistId === playlistId);
    if (!o || !o.members.length) return null;
    const summary = this.ownedSummary(o);
    const byId = new Map(summary.people.map((p) => [p.id, p]));
    const addedBy: Record<string, CirclePerson> = {};
    for (const [item, by] of o.addedBy) {
      const p = byId.get(by);
      if (p) addedBy[item] = p;
    }
    return { summary, addedBy };
  }

  inviteLink(playlistId: string, role: "collaborator" | "listener"): InviteLink {
    this.need("playlist");
    const o = this.ownedFor(playlistId);
    const l = this.link("collab");
    // The demo: someone opens it after a moment.
    if (this.h.demo()) this.h.later(4000, () => this.personJoins(o.playlistId, ["Casey", "Morgan", "Avery"][this.invites % 3], role));
    return l;
  }

  invitePerson(playlistId: string, key: string, role: "collaborator" | "listener"): void {
    this.need("playlist");
    const o = this.ownedFor(playlistId);
    this.link("collab");
    const who = this.h.people().find((p) => p.key === key);
    if (!who) throw err("notFound", "They're not on your server any more.");
    this.h.later(this.h.demo() ? 2500 : 20, () => this.personJoins(o.playlistId, who.name, role, who.avatar));
  }

  /** Test hook: someone redeems an invite to your playlist. */
  personJoins(playlistId: string, name: string, role: CollabRole, av: string | null = null) {
    const o = this.ownedFor(playlistId);
    if (o.members.some((m) => m.p.name === name)) return;
    o.members.push({ p: { ...person(name, null, role), avatar: av }, role });
    this.h.notice(`${name} joined ${this.h.playlist(playlistId)?.title ?? "your playlist"}.`);
    this.changed();
    // The demo: a collaborator adds a song a little later.
    if (this.h.demo() && role === "collaborator") {
      const t = this.h.tracks()[(name.length * 31) % this.h.tracks().length];
      this.h.later(6000, () => this.memberAdds(name, t.id, playlistId));
    }
  }

  /** Test hook: a collaborator's add arrives through the relay; you apply it. */
  memberAdds(name: string, trackId: string, playlistId = MOCK_OWNED_SHARED) {
    const o = this.owned.find((x) => x.playlistId === playlistId);
    const m = o?.members.find((x) => x.p.name === name);
    if (!o || !m || m.role !== "collaborator") return;
    this.h.add(playlistId, [trackId]);
    const items = this.h.itemIds(playlistId);
    o.addedBy.set(items[items.length - 1], m.p.id);
    this.changed();
  }

  setRole(playlistId: string, personId: string, role: "collaborator" | "listener") {
    const o = this.ownedFor(playlistId);
    const m = o.members.find((x) => x.p.id === personId);
    if (!m) throw err("notFound", "They're not on this playlist any more.");
    m.role = role;
    m.p = { ...m.p, role };
    this.changed();
  }

  removePerson(playlistId: string, personId: string) {
    const o = this.ownedFor(playlistId);
    if (!o.members.some((x) => x.p.id === personId)) throw err("notFound", "They're not on this playlist any more.");
    o.members = o.members.filter((x) => x.p.id !== personId);
    this.changed();
  }

  setPublic(playlistId: string, on: boolean) {
    const o = this.ownedFor(playlistId);
    o.public = on;
    if (!on) o.members = o.members.filter((m) => m.role !== "follower");
    this.changed();
  }

  private joinedFor(id: string): Joined {
    const j = this.joined.find((x) => x.id === id);
    if (!j) throw err("notFound", "This playlist isn't shared with you any more.");
    return j;
  }

  page(id: string): SharedPage {
    const j = this.joinedFor(id);
    const rows = [...j.rows.filter((r) => !j.removing.has(r.key)), ...j.pending];
    return {
      summary: this.joinedSummary(j),
      description: j.description,
      durationMs: rows.reduce((s, r) => s + r.durationMs, 0),
      rows: rows.map((r) => ({ ...r, removable: !r.pending && j.role === "collaborator" && !!r.by?.you })),
    };
  }

  /** Your edit reaches the owner, who applies it and sends it back. */
  private deliver(j: Joined) {
    if (this.offline) {
      this.tries++;
      this.h.later(this.h.demo() ? 3000 : 60, () => this.deliver(j));
      if (this.tries === 3) this.changed();
      return;
    }
    this.tries = 0;
    j.rows = [...j.rows.filter((r) => !j.removing.has(r.key)), ...j.pending.map((r) => ({ ...r, pending: false, key: r.key.replace("pending-", "k") }))];
    j.pending = [];
    j.removing.clear();
    this.changed();
  }

  add(id: string, trackIds: string[]) {
    const j = this.joinedFor(id);
    if (j.gone) throw err("invalid", "This playlist isn't available any more.");
    if (j.role !== "collaborator") throw err("invalid", "Only collaborators can add songs to this playlist.");
    const you = j.people.find((p) => p.you)!;
    for (const tid of trackIds) {
      const t = this.h.track(tid);
      if (t) j.pending.push({ ...this.row(t, you, j.rows.length + j.pending.length), key: `pending-${tid}-${seq++}`, pending: true, addedAt: Math.floor(Date.now() / 1000) });
    }
    this.changed();
    this.h.later(this.h.demo() ? 900 : 30, () => this.deliver(j));
  }

  remove(id: string, keys: string[]) {
    const j = this.joinedFor(id);
    for (const k of keys) {
      const r = j.rows.find((x) => x.key === k);
      if (r && !r.by?.you) throw err("invalid", "You can only remove songs you added.");
      j.removing.add(k);
    }
    this.changed();
    this.h.later(this.h.demo() ? 900 : 30, () => this.deliver(j));
  }

  leave(id: string) {
    const j = this.joinedFor(id);
    if (j.public && !j.gone) {
      j.role = "follower";
      j.people = j.people.map((p) => (p.you ? { ...p, role: "follower" } : p));
    } else {
      this.joined = this.joined.filter((x) => x !== j);
    }
    this.changed();
  }

  setInLibrary(id: string, on: boolean) {
    this.joinedFor(id).inLibrary = on;
    this.changed();
  }

  join(link: string): JoinResult {
    const kind = link.startsWith("platter://collab/") ? "playlist" : link.startsWith("platter://blend/") ? "blend" : null;
    if (!kind) throw err("invalid", "That isn't a playlist or Blend invite.");
    this.need(kind);
    const fail = this.nextFail;
    this.nextFail = null;
    if (fail === "expired") throw err("invalid", "That invite has expired. Ask for a new one.");
    if (fail === "used") throw err("invalid", "That invite was already used. Ask for a new one.");
    if (fail === "relay" || this.health === "unreachable") throw err("network", "Couldn't reach the relay to join. Try again in a while.");
    if (this.waits) {
      this.joining++;
      this.changed();
      return { state: "waiting", kind };
    }
    return this.joinNow(kind);
  }

  private joinNow(kind: "playlist" | "blend"): JoinResult {
    if (kind === "blend") {
      const b: Blend = { id: `blend:${String(seq++).padStart(32, "a")}`, others: [person("Avery", 4)], hidden: new Set(), day: 0 };
      this.blends.push(b);
      this.changed();
      return { state: "joined", id: b.id, kind };
    }
    const owner = person("Avery", 4, "owner");
    const you = { ...person(this.h.me().name, 0, "listener", true), id: this.youId };
    const tracks = this.fx.tracks.filter((_, i) => i % 41 === 7).slice(0, 6);
    const j: Joined = {
      id: `shared:${String(seq++).padStart(32, "b")}`,
      title: "Avery's Picks",
      description: null,
      owner,
      role: "listener",
      public: false,
      people: [owner, you],
      rows: tracks.map((t, i) => this.row(t, owner, i)),
      gone: false,
      inLibrary: true,
      pending: [],
      removing: new Set(),
    };
    this.joined.push(j);
    this.changed();
    return { state: "joined", id: j.id, kind };
  }

  accept(id: string): JoinResult {
    const inc = this.incoming.find((i) => i.id === id);
    if (!inc) throw err("notFound", "That invite isn't here any more.");
    this.incoming = this.incoming.filter((i) => i !== inc);
    return this.joinNow(inc.kind);
  }

  dismiss(id: string) {
    this.incoming = this.incoming.filter((i) => i.id !== id);
    this.changed();
  }

  // ---------------------------------------------------------------- Blends

  private blendFor(id: string): Blend {
    const b = this.blends.find((x) => x.id === id);
    if (!b) throw err("notFound", "This Blend isn't here any more.");
    return b;
  }

  /** Taking turns: you, then each friend, from the fixture library. */
  private blendRows(b: Blend): { track: Track; by: CirclePerson }[] {
    const people = [this.meInBlend(), ...b.others];
    const pool = this.h.tracks().filter((t) => !b.hidden.has(t.id));
    const out: { track: Track; by: CirclePerson }[] = [];
    const seen = new Set<string>();
    for (let i = 0; out.length < 50 && i < pool.length * 2; i++) {
      const by = people[out.length % people.length];
      const t = pool[(i * 53 + b.day * 7 + by.name.length * 11) % pool.length];
      if (seen.has(t.id)) continue;
      seen.add(t.id);
      out.push({ track: t, by });
    }
    return out;
  }

  private meInBlend(): CirclePerson {
    const me = this.h.me();
    return { id: this.youId, name: me.name, avatar: me.avatar, role: null, you: true, added: 0 };
  }

  blendPage(id: string): BlendPage {
    const b = this.blendFor(id);
    const rows = this.blendRows(b);
    const missing: BlendMissing[] = b.others.slice(0, 1).map((o) => ({ title: "Harbour Lights", artist: "The Night Ferries", album: "Coastline", releaseGroupId: "00000000-0000-4000-8000-000000000030", by: o }));
    return { summary: { ...this.blendSummary(b), trackCount: rows.length }, rows, missing, durationMs: rows.reduce((s, r) => s + r.track.durationMs, 0) };
  }

  blendCreate(keys: string[]): BlendCreated {
    this.need("blend");
    if (keys.length + 1 > 10) throw err("invalid", "A Blend takes up to 10 people, you included.");
    const invite = this.link("blend");
    const b: Blend = { id: `blend:${String(seq++).padStart(32, "c")}`, others: [], hidden: new Set(), day: 0 };
    this.blends.push(b);
    this.changed();
    const people = this.h.people();
    keys.forEach((k, i) => {
      const who = people.find((p) => p.key === k);
      if (who) this.h.later(this.h.demo() ? 2500 + i * 1500 : 20, () => this.blendJoins(b.id, who.name, who.avatar));
    });
    return { id: b.id, invite };
  }

  /** Test hook: someone joins a Blend. */
  blendJoins(id: string, name: string, av: string | null = null) {
    const b = this.blends.find((x) => x.id === id);
    if (!b || b.others.some((o) => o.name === name) || b.others.length >= 9) return;
    b.others.push({ ...person(name, null), avatar: av });
    this.h.notice(`${name} joined your Blend.`);
    this.changed();
  }

  blendInviteLink(id: string): InviteLink {
    this.need("blend");
    const b = this.blendFor(id);
    if (b.others.length >= 9) throw err("invalid", "This Blend is full. A Blend takes up to 10 people.");
    return this.link("blend");
  }

  blendInvitePerson(id: string, key: string) {
    this.blendInviteLink(id);
    const who = this.h.people().find((p) => p.key === key);
    if (who) this.h.later(this.h.demo() ? 2500 : 20, () => this.blendJoins(id, who.name, who.avatar));
  }

  blendHide(id: string, trackId: string) {
    this.blendFor(id).hidden.add(trackId);
    this.changed();
  }

  blendRefresh(id: string): BlendPage {
    this.blendFor(id).day++;
    this.changed();
    return this.blendPage(id);
  }

  blendLeave(id: string) {
    this.blendFor(id);
    this.blends = this.blends.filter((b) => b.id !== id);
    this.changed();
  }

  // ---------------------------------------------------------------- test hooks

  relay(health: CircleView["relay"]) {
    this.health = health;
    this.changed();
  }

  ownerOffline(on: boolean) {
    this.offline = on;
    if (!on) this.tries = 0;
    this.changed();
  }

  /** The owner of the playlist shared with you deletes it. */
  ownerDeletes(id = MOCK_SHARED_ID) {
    const j = this.joined.find((x) => x.id === id);
    if (!j) return;
    j.gone = true;
    this.h.notice(`${j.owner.name} deleted ${j.title}.`);
    this.changed();
  }

  failNextJoin(why: "expired" | "used" | "relay") {
    this.nextFail = why;
  }

  joinWaits(on: boolean) {
    this.waits = on;
  }

  ring(from: string, kind: "playlist" | "blend", title: string) {
    this.incoming.push({ id: `in${seq++}`, from, kind, title, at: Date.now() });
    this.h.notice(kind === "playlist" ? `${from} invited you to the playlist ${title}.` : `${from} invited you to a Blend.`);
    this.changed();
  }

  /** Playlists you can add to as a collaborator, as playlists. */
  collaboratorPlaylists(): Playlist[] {
    return this.joined
      .filter((j) => j.role === "collaborator" && !j.gone)
      .map((j) => ({ id: j.id, title: j.title, smart: false, trackCount: j.rows.length, durationMs: 0, thumb: null, summary: j.description, updatedAt: 0 }));
  }

  /** Removing a playlist you own drops its sharing (members see it gone). */
  playlistDeleted(id: string) {
    this.owned = this.owned.filter((o) => o.playlistId !== id);
    this.changed();
  }
}
