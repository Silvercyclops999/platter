/**
 * Navidrome and other Subsonic servers in the mock, with hooks to fake each
 * way adding or using one can go wrong.
 */
import type { AppError, Caps, ResumeOffer, SavedServer, ServerCaps, ServerLibrary, SubsonicCheck } from "../types";
import { ALL_CAPS } from "../caps";

/** Ways the fake server can fail. */
export type SubsonicFailure = "wrongUrl" | "notSubsonic" | "keyRefused" | "noExtensions" | null;

const err = (kind: AppError["kind"], message: string): AppError => ({ kind, message });

export const WRONG_URL = "Platter can't reach a server at that address. Check it and try again.";
export const NOT_SUBSONIC = "That address doesn't answer like a Navidrome or other Subsonic server.";
export const WRONG_PASSWORD = "That user name and password didn't work.";
export const KEY_REFUSED = "The server didn't accept that API key. Make a new one on the server and try again.";
export const NO_API_KEYS = "This server doesn't take API keys. Sign in with your user name and password instead.";
export const NOT_THIS_SERVER =
  "The server at that address isn't this one: it doesn't have this server's songs. Check the address, or add it as a new server.";

/** What a Subsonic server can do, as the core reports it. */
export function subsonicCaps(open: boolean): Caps {
  const c = Object.fromEntries(Object.keys(ALL_CAPS).map((k) => [k, false])) as Caps;
  for (const k of ["likeDates", "playlistMove", "playlistSummary", "transcode", "serverRadio", "friendActivity", "resumeQueue", "backdatedScrobble"] as const)
    c[k] = true;
  for (const k of ["loudness", "releaseTypes", "lyrics", "syncedLyrics"] as const) c[k] = open;
  return c;
}

export class MockSubsonic {
  /** The next check or sign-in fails this way. */
  failure: SubsonicFailure = null;
  /** Addresses that answer as another server (tests). */
  others = new Set<string>();
  /** The account's rights on servers added from now on. */
  canDownload = true;
  servers: SavedServer[] = [];
  libraries: Record<string, ServerLibrary[]> = {};
  /** Queues saved on each server by another app. */
  queues: Record<string, ResumeOffer> = {};
  /** Queues played (tests). */
  played: string[] = [];
  private seq = 0;

  check(url: string): SubsonicCheck {
    const u = url.trim();
    if (!u) throw err("invalid", "Type your server's address.");
    if (this.failure === "wrongUrl") throw err("invalid", WRONG_URL);
    if (this.failure === "notSubsonic") throw err("invalid", NOT_SUBSONIC);
    const tidy = (u.includes("://") ? u : `http://${u}`).replace(/\/(rest|app)(\/.*)?$/, "").replace(/#.*$/, "").replace(/\/+$/, "");
    const open = this.failure !== "noExtensions";
    const host = tidy.replace(/^https?:\/\//, "").split(/[:/]/)[0];
    const insecure = tidy.startsWith("http://") && host.includes(".") && !/\.(local|lan|home|internal)$/.test(host) && !/^(10|127|192\.168)\./.test(host);
    return { url: tidy, name: open ? `Navidrome on ${host}` : host, version: open ? "0.58.0" : null, openSubsonic: open, apiKeys: open, insecure };
  }

  signIn(url: string, creds: { apiKey?: string; user?: string; password?: string }, tag?: string): SavedServer {
    const c = this.check(url);
    if (creds.apiKey) {
      if (!c.apiKeys) throw err("invalid", NO_API_KEYS);
      if (this.failure === "keyRefused" || creds.apiKey !== "good-key") throw err("auth", KEY_REFUSED);
      return this.add(c, "me", tag);
    }
    if (!creds.user?.trim()) throw err("invalid", "Type your user name.");
    if (creds.password !== "right") throw err("auth", WRONG_PASSWORD);
    return this.add(c, creds.user.trim(), tag);
  }

  /** Like the core: the same server at a new address keeps its tag. */
  changeAddress(tag: string, url: string): SavedServer {
    const s = this.servers.find((x) => x.tag === tag);
    if (!s) throw err("notFound", "That server isn't in Platter any more.");
    const c = this.check(url);
    if (this.others.has(c.url)) throw err("invalid", NOT_THIS_SERVER);
    s.url = c.url;
    s.state = "online";
    return s;
  }

  private add(c: SubsonicCheck, user: string, keep?: string): SavedServer {
    const again = keep ? this.servers.find((s) => s.tag === keep) : undefined;
    if (again) {
      if (this.others.has(c.url)) throw err("invalid", NOT_THIS_SERVER);
      again.url = c.url;
      again.state = "online";
      return again;
    }
    const known = this.servers.find((s) => s.url === c.url);
    if (known) {
      known.state = "online";
      return known;
    }
    this.seq += 1;
    const tag = `5ab5000${this.seq}`;
    const s: SavedServer = { tag, kind: "subsonic", name: c.name, url: c.url, user, primary: false, state: "online", canTranscode: true, canDownload: this.canDownload };
    this.servers.push(s);
    this.libraries[tag] = [{ id: "1", title: "Music", included: true }];
    return s;
  }

  /** Test hook: the key or password stopped working. */
  revoke(tag: string) {
    const s = this.servers.find((x) => x.tag === tag);
    if (s) s.state = "signInAgain";
  }

  owns(tag: string) {
    return this.servers.some((s) => s.tag === tag);
  }

  remove(tag: string) {
    this.servers = this.servers.filter((s) => s.tag !== tag);
    delete this.libraries[tag];
    delete this.queues[tag];
  }

  setLibraries(tag: string, ids: string[] | null) {
    if (ids && ids.length === 0) throw err("invalid", "Include at least one library, or remove the server.");
    const libs = this.libraries[tag];
    if (!libs) throw err("notFound", "That server isn't in Platter any more.");
    this.libraries[tag] = libs.map((l) => ({ ...l, included: ids === null || ids.includes(l.id) }));
  }

  caps(): ServerCaps[] {
    return this.servers.map((s) => ({ tag: s.tag, kind: s.kind, name: s.name, primary: s.primary, caps: subsonicCaps(s.name.startsWith("Navidrome")) }));
  }

  offers(): ResumeOffer[] {
    return this.servers.map((s) => this.queues[s.tag]).filter((q): q is ResumeOffer => !!q);
  }

  play(tag: string) {
    if (!this.queues[tag]) throw err("notFound", "There's no saved queue on that server.");
    this.played.push(tag);
  }

  reset() {
    this.failure = null;
    this.canDownload = true;
    this.servers = [];
    this.libraries = {};
    this.queues = {};
    this.played = [];
    this.seq = 0;
  }
}
