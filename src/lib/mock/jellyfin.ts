/**
 * Servers added by address (Jellyfin) in the mock, with hooks to fake each
 * way adding or using one can go wrong.
 */
import type { AppError, QuickCode, QuickPoll, SavedServer, ServerCheck, ServerLibrary } from "../types";
import { plexCaps } from "./servers";
import type { Caps, ServerCaps } from "../types";
import { ALL_CAPS } from "../caps";

/** Ways the fake server can fail. */
export type JellyfinFailure = "wrongUrl" | "notJellyfin" | "oldVersion" | "quickConnectOff" | "expired" | null;

const err = (kind: AppError["kind"], message: string): AppError => ({ kind, message });

export const WRONG_URL = "Platter can't reach a server at that address. Check it and try again.";
export const NOT_JELLYFIN = "That address doesn't answer like a Jellyfin server.";
export const QUICK_CONNECT_OFF = "Quick Connect is turned off on this Jellyfin server. Sign in with your user name and password instead.";
export const EXPIRED = "That code ran out. Start again for a new one.";
export const WRONG_PASSWORD = "That user name and password didn't work.";

/** What a Jellyfin account can do, as the core reports it. */
export function jellyfinCaps(admin: boolean, transcode: boolean): Caps {
  const c = Object.fromEntries(Object.keys(ALL_CAPS).map((k) => [k, false])) as Caps;
  for (const k of ["liveUpdates", "incrementalSync", "playlistMove", "lyrics", "syncedLyrics", "serverRadio", "loudness", "backdatedScrobble"] as const)
    c[k] = true;
  c.transcode = transcode;
  c.friendActivity = admin;
  c.realFilePaths = admin;
  return c;
}

export class MockJellyfin {
  /** The next check, sign-in or code fails this way. */
  failure: JellyfinFailure = null;
  /** Polls before the code counts as approved. */
  approveAfter = 1;
  /** The account's rights on servers added from now on. */
  canTranscode = true;
  canDownload = true;
  servers: SavedServer[] = [];
  libraries: Record<string, ServerLibrary[]> = {};
  private pending: { url: string; polls: number } | null = null;
  private seq = 0;

  check(url: string): ServerCheck {
    const u = url.trim();
    if (!u) throw err("invalid", "Type your Jellyfin server's address.");
    if (this.failure === "wrongUrl") throw err("invalid", WRONG_URL);
    if (this.failure === "notJellyfin") throw err("invalid", NOT_JELLYFIN);
    if (this.failure === "oldVersion")
      throw err("invalid", "This Jellyfin server is version 10.8.13. Platter needs 10.9 or later; update the server to use it here.");
    const tidy = (u.includes("://") ? u : `http://${u}`).replace(/\/web(\/index\.html)?(#.*)?$/, "").replace(/\/+$/, "");
    return { url: tidy, name: "Den", version: "10.10.3", quickConnect: this.failure !== "quickConnectOff" };
  }

  quickStart(url: string): QuickCode {
    const c = this.check(url);
    if (!c.quickConnect) throw err("invalid", QUICK_CONNECT_OFF);
    this.pending = { url: c.url, polls: 0 };
    return { code: "482913", serverName: c.name };
  }

  quickPoll(): QuickPoll {
    if (!this.pending || this.failure === "expired") {
      this.pending = null;
      throw err("invalid", EXPIRED);
    }
    this.pending.polls += 1;
    if (this.pending.polls <= this.approveAfter) return { state: "waiting" };
    const server = this.add(this.pending.url);
    this.pending = null;
    return { state: "done", server };
  }

  quickCancel() {
    this.pending = null;
  }

  password(url: string, user: string, password: string): SavedServer {
    const c = this.check(url);
    if (!user.trim()) throw err("invalid", "Type your Jellyfin user name.");
    if (password !== "right") throw err("auth", WRONG_PASSWORD);
    return this.add(c.url, user.trim());
  }

  private add(url: string, user = "me"): SavedServer {
    const known = this.servers.find((s) => s.url === url);
    if (known) {
      known.state = "online";
      return known;
    }
    this.seq += 1;
    const tag = `f00d000${this.seq}`;
    const s: SavedServer = {
      tag,
      kind: "jellyfin",
      name: this.seq === 1 ? "Den" : `Den ${this.seq}`,
      url,
      user,
      primary: false,
      state: "online",
      canTranscode: this.canTranscode,
      canDownload: this.canDownload,
    };
    this.servers.push(s);
    this.libraries[tag] = [
      { id: "v1", title: "Music", included: true },
      { id: "v3", title: "Live tapes", included: true },
    ];
    return s;
  }

  /** Test hook: the server revoked the sign-in. */
  revoke(tag: string) {
    const s = this.servers.find((x) => x.tag === tag);
    if (s) s.state = "signInAgain";
  }

  remove(tag: string) {
    this.servers = this.servers.filter((s) => s.tag !== tag);
    delete this.libraries[tag];
  }

  setLibraries(tag: string, ids: string[] | null) {
    if (ids && ids.length === 0) throw err("invalid", "Include at least one library, or remove the server.");
    const libs = this.libraries[tag];
    if (!libs) throw err("notFound", "That server isn't in Platter any more.");
    this.libraries[tag] = libs.map((l) => ({ ...l, included: ids === null || ids.includes(l.id) }));
  }

  caps(): ServerCaps[] {
    return this.servers.map((s) => ({ tag: s.tag, kind: s.kind, name: s.name, primary: s.primary, caps: jellyfinCaps(false, s.canTranscode) }));
  }

  reset() {
    this.failure = null;
    this.approveAfter = 1;
    this.canTranscode = true;
    this.canDownload = true;
    this.servers = [];
    this.libraries = {};
    this.pending = null;
    this.seq = 0;
  }
}

/** The Plex server's record as Settings lists it. */
export function plexRecord(name: string, primary: boolean): SavedServer {
  return { tag: "0a1b2c3d", kind: "plex", name, url: null, user: null, primary, state: null, canTranscode: true, canDownload: true };
}

export { plexCaps };
