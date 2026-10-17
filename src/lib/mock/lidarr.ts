import type {
  AppError,
  LidarrFinding,
  LidarrMissing,
  LidarrPickInput,
  LidarrPicked,
  LidarrPreview,
  LidarrReport,
  LidarrRequest,
  LidarrGetOutcome,
  LidarrRequestInput,
  LidarrRequestState,
  LidarrStatus,
  Settings,
} from "../types";

/**
 * Lidarr's setup check in the browser mock. `?lidarr=` picks what the faked
 * Lidarr looks like: `demo` (one red, two amber; the default), `clear`,
 * `empty` (an empty root folder to vouch for), `away` (only the away address
 * answers), `unreachable`, `badKey` or `old`. `ready` is set up with a
 * green check (artist pages list missing releases, requests send);
 * `offline` is the same with Lidarr out of reach (requests wait).
 */
export type LidarrDemo = "demo" | "clear" | "empty" | "away" | "unreachable" | "badKey" | "old" | "ready" | "offline";
export const LIDARR_DEMOS: LidarrDemo[] = ["demo", "clear", "empty", "away", "unreachable", "badKey", "old", "ready", "offline"];

/** Mock MusicBrainz: official releases each artist lacks, by artist title. */
const MISSING_BY_DEFAULT = [
  { releaseGroupId: "6f1c0c55-0000-4000-8000-000000000001", title: "Paper Lanterns", kind: "album", year: 2014 },
  { releaseGroupId: "6f1c0c55-0000-4000-8000-000000000002", title: "Low Tide EP", kind: "ep", year: 2017 },
  { releaseGroupId: "6f1c0c55-0000-4000-8000-000000000003", title: "Northern Rooms", kind: "album", year: 2022 },
];
/** The small cover path the core gives a release group (null: the archive has none). */
export const coverThumb = (rgid: string) => `/ext/https://coverartarchive.org/release-group/${rgid}/front-250`;

/**
 * Mock MusicBrainz for songs on the Missing list: the album each comes from,
 * by folded title. Glass Orchard only exists as a bootleg.
 */
const SONG_ALBUMS: Record<string, { id: string; title: string; year: string } | "bootleg"> = {
  "paper boats": { id: "7a2d0c55-0000-4000-8000-000000000011", title: "Low Water", year: "2016" },
  undertow: { id: "7a2d0c55-0000-4000-8000-000000000011", title: "Low Water", year: "2016" },
  "neon driveway": { id: "7a2d0c55-0000-4000-8000-000000000012", title: "Night Errands", year: "2019" },
  "thin ice waltz": { id: "7a2d0c55-0000-4000-8000-000000000013", title: "Winter Rooms", year: "2012" },
  "field notes": { id: "7a2d0c55-0000-4000-8000-000000000014", title: "Field Notes", year: "2020" },
  "glass orchard": "bootleg",
};

export const isLidarrDemo = (v: string | null): v is LidarrDemo => !!v && (LIDARR_DEMOS as string[]).includes(v);

const invalid = (message: string): AppError => ({ kind: "invalid", message });

const PRIVATE = /^(localhost|127\.|10\.|192\.168\.|172\.(1[6-9]|2\d|3[01])\.|169\.254\.|100\.(6[4-9]|[7-9]\d|1[01]\d|12[0-7])\.|\[?::1\]?$|\[?f[cd])|^[^.]+$|\.(local|lan|home|internal|home\.arpa)$/i;

/** A plain http address to a host outside the home network. */
export function unencrypted(url: string): boolean {
  try {
    const u = new URL(url);
    return u.protocol === "http:" && !PRIVATE.test(u.hostname);
  } catch {
    return false;
  }
}

const RESERVED = ["x-api-key", "host", "content-length", "content-type", "transfer-encoding", "connection", "user-agent", "accept-encoding"];

/** Same rules as the core: `Name: value` lines, up to 5. */
export function parseHeaders(text: string): [string, string][] {
  const out: [string, string][] = [];
  for (const raw of text.split("\n")) {
    const line = raw.trim();
    if (!line) continue;
    const at = line.indexOf(":");
    const name = at < 0 ? "" : line.slice(0, at).trim();
    const value = at < 0 ? "" : line.slice(at + 1).trim();
    if (!/^[A-Za-z0-9_-]{1,64}$/.test(name) || !value || !/^[\t -~]+$/.test(value)) {
      throw invalid("Write each header on its own line as Name: value.");
    }
    if (RESERVED.includes(name.toLowerCase())) throw invalid(`Platter sets ${name} itself; leave it out.`);
    if (out.some(([n]) => n.toLowerCase() === name.toLowerCase())) throw invalid(`${name} is there twice.`);
    out.push([name, value]);
  }
  if (out.length > 5) throw invalid("Up to 5 extra headers.");
  return out;
}

const ok = (id: string, title: string, detail = ""): LidarrFinding => ({ id, level: "ok", title, detail, fix: null, emptyRoot: null, protectedOnly: false, action: null });

const OK_SETTINGS: LidarrFinding[] = [
  ok("renameTracks", "Track renaming is off"),
  ok("writeAudioTags", "Lidarr doesn't write tags into files"),
  ok("scrubAudioTags", "Lidarr keeps tags it doesn't know"),
  ok("deleteEmptyFolders", "Lidarr leaves empty folders alone"),
  ok("recycleBin", "Lidarr keeps removed files in a recycling bin"),
  ok("upgradeAllowed", "Lidarr doesn't replace files with better copies"),
  ok("importLists", "No import lists add artists on their own"),
];

const ROOT_FIX =
  "In Lidarr, open Settings > Media Management > Root Folders and add a separate, empty folder for downloads (for example a Lidarr folder next to your music, added to the Plex music library as a second folder). Then remove this root folder from Lidarr. Platter never changes Lidarr's folders itself.";

/** Albums a check refuses in the mock (by release group), with why. */
export const REFUSED: Record<string, string> = {
  "c0ffee11-0000-4000-8000-000000000062":
    "Lowlight shares its title with an unreleased album in your library, so Platter won't ask Lidarr for it. Buy it from a store instead.",
};

export class MockLidarr {
  private hasKey = false;
  private headers: [string, string][] = [];
  private confirmed = new Set<string>();
  private lastPassed = false;
  private typedOnce = false;
  requests: LidarrRequest[] = [];

  constructor(private demo: LidarrDemo = "demo") {
    if (demo === "ready" || demo === "offline") {
      this.hasKey = true;
      this.lastPassed = true;
    }
  }

  /** Whether the faked Lidarr answers now. */
  private reachable(s: Settings) {
    return this.demo !== "offline" && this.demo !== "unreachable" && !(this.demo === "away" && !s.lidarrAwayUrl.trim());
  }

  /** Test hook: switch what the faked Lidarr looks like. */
  setDemo(demo: LidarrDemo) {
    this.demo = demo;
  }

  /** Test hook: no key, no headers, nothing confirmed, no requests. */
  reset(demo: LidarrDemo = "demo") {
    this.demo = demo;
    this.hasKey = demo === "ready" || demo === "offline";
    this.lastPassed = this.hasKey;
    this.typedOnce = false;
    this.headers = [];
    this.confirmed.clear();
    this.requests = [];
  }

  /** Test hook: the first request has been typed already. */
  setTypedOnce(v: boolean) {
    this.typedOnce = v;
  }

  private setUp(s: Settings) {
    return (!!s.lidarrUrl.trim() || !!s.lidarrAwayUrl.trim()) && this.hasKey;
  }

  missing(s: Settings, _artistId: string): LidarrMissing {
    if (!s.showMissingReleases || !this.setUp(s) || !this.lastPassed) return { shown: false, items: [] };
    return {
      shown: true,
      items: MISSING_BY_DEFAULT.map((m) => ({
        ...m,
        request: this.requests.find((r) => r.releaseGroupId === m.releaseGroupId)?.state ?? null,
        // The EP has no cover in the archive: its card keeps the placeholder.
        thumb: m.kind === "ep" ? null : coverThumb(m.releaseGroupId),
      })),
    };
  }

  /** The release group to request for a song or album (official ones only), or why there's none. */
  pick(s: Settings, input: LidarrPickInput): LidarrPicked {
    if (!this.setUp(s)) throw invalid("Set up Lidarr in Settings first.");
    const key = input.title.trim().replace(/\s+/g, " ").toLowerCase();
    const hit = input.song ? SONG_ALBUMS[key] : Object.values(SONG_ALBUMS).find((g) => g !== "bootleg" && g.title.toLowerCase() === key);
    if (hit === "bootleg") {
      return {
        group: null,
        artist: input.artist,
        refusal: { kind: "notOfficial" },
        reason: "MusicBrainz lists this only as a bootleg or promo, not an official release. Platter never asks Lidarr for those.",
      };
    }
    if (!hit) {
      return { group: null, artist: input.artist, refusal: { kind: "notFound" }, reason: "MusicBrainz doesn't have an official release for this, so Lidarr can't get it." };
    }
    return { group: { id: hit.id, title: hit.title, primary: "album", secondary: [], firstRelease: hit.year }, artist: input.artist, refusal: null, reason: null };
  }

  private destination(artist: string) {
    return `/downloads/lidarr/${artist}`;
  }

  preview(s: Settings, input: LidarrRequestInput): LidarrPreview {
    if (!this.setUp(s)) throw invalid("Set up Lidarr in Settings first.");
    const reachable = this.reachable(s);
    if (reachable && this.demo === "demo") {
      throw invalid("Platter won't send Lidarr anything while its setup check has red items: Lidarr has a protected artist in your Plex music. Fix them in Lidarr, then check again in Settings.");
    }
    return {
      releaseGroupId: input.releaseGroupId,
      title: input.title,
      artist: input.artist,
      destination: this.destination(input.artist),
      destinationIsGuess: !reachable,
      needsTyped: !this.typedOnce,
      existing: null,
      reachable,
      route: reachable ? "home" : null,
      addsArtist: !this.requests.some((r) => r.artist === input.artist && r.state !== "queued"),
      protected: false,
    };
  }

  request(s: Settings, input: LidarrRequestInput, now: number): LidarrRequest {
    const p = this.preview(s, input);
    const fold = (t: string) => t.trim().replace(/\s+/g, " ").toLowerCase();
    if (p.needsTyped && (!input.typed || fold(input.typed) !== fold(input.title))) {
      throw invalid(`Type the album's name, ${input.title}, to send your first request.`);
    }
    const had = this.requests.find((r) => r.releaseGroupId === input.releaseGroupId && r.state !== "failed");
    if (had) return had;
    const state: LidarrRequestState = p.reachable ? "searching" : "queued";
    const r: LidarrRequest = {
      id: `req-${this.requests.length + 1}`,
      releaseGroupId: input.releaseGroupId,
      title: input.title,
      artist: input.artist,
      artistId: input.artistId ?? null,
      state,
      destination: p.destination,
      lidarrAlbumId: p.reachable ? 100 + this.requests.length : null,
      addedArtist: p.reachable && p.addsArtist,
      alreadyInLidarr: false,
      createdAt: now,
      updatedAt: now,
      message: p.reachable ? null : "Sends when Platter can reach Lidarr.",
      notes: p.reachable && p.addsArtist ? [`Added ${input.artist} to Lidarr with only this album to download.`] : [],
      auto: false,
      protected: false,
    };
    this.requests = [r, ...this.requests];
    this.typedOnce = true;
    return r;
  }

  /** One-click Get: the request, or why it needs the user. */
  get(s: Settings, input: LidarrRequestInput, now: number): LidarrGetOutcome {
    if (!this.typedOnce && !input.typed) {
      return { record: null, needs: { kind: "firstTime", reason: "Your first request is confirmed by typing the album's name, once." } };
    }
    if (REFUSED[input.releaseGroupId]) return { record: null, needs: { kind: "refused", reason: REFUSED[input.releaseGroupId] } };
    try {
      return { record: this.request(s, input, now), needs: null };
    } catch (e) {
      const err = e as { kind?: string; message?: string };
      if (err.kind === "invalid" || err.kind === "notFound") return { record: null, needs: { kind: "refused", reason: err.message ?? "" } };
      throw e;
    }
  }

  /** Test hook: what Lidarr is doing with a request now. */
  advance(releaseGroupId: string, state: LidarrRequestState, progress: number | null = null) {
    this.requests = this.requests.map((r) =>
      r.releaseGroupId === releaseGroupId
        ? { ...r, state, progress, message: state === "notFound" ? "Nothing turned up on your indexers yet. Search again, or try another edition." : null }
        : r,
    );
  }

  /** "Search again": one search now, the schedule starts over. */
  searchAgain(id: string, now: number): LidarrRequest {
    const r = this.requests.find((x) => x.id === id);
    if (!r) throw { kind: "notFound", message: "That request isn't on this device any more." };
    if (r.state !== "searching" && r.state !== "notFound") throw invalid("Lidarr is already working on this album.");
    const next = { ...r, state: "searching" as const, message: null, searchedAt: now, searches: 1, nextSearchAt: now + 3600, updatedAt: now };
    this.requests = this.requests.map((x) => (x.id === id ? next : x));
    this.searchesAgain.push(id);
    return next;
  }

  /** Every "Search again" sent (tests read it). */
  readonly searchesAgain: string[] = [];

  list(): LidarrRequest[] {
    return [...this.requests];
  }

  cancel(id: string): LidarrRequest[] {
    const r = this.requests.find((x) => x.id === id);
    if (r && !(r.state === "queued" || r.state === "imported" || r.state === "failed" || r.alreadyInLidarr)) {
      throw invalid("Lidarr already has this request. Remove the album in Lidarr to stop it.");
    }
    this.requests = this.requests.filter((x) => x.id !== id);
    return this.list();
  }

  status(s: Settings): LidarrStatus {
    return {
      urlSet: !!s.lidarrUrl.trim(),
      awaySet: !!s.lidarrAwayUrl.trim(),
      hasKey: this.hasKey,
      headerNames: this.headers.map(([n]) => n),
      unencrypted: [s.lidarrUrl, s.lidarrAwayUrl].filter((u) => u && unencrypted(u)),
      ready: this.setUp(s) && this.lastPassed,
    };
  }

  setKey(s: Settings, key: string | null): LidarrStatus {
    const k = (key ?? "").trim();
    if (k && !/^[A-Za-z0-9]{1,200}$/.test(k)) throw invalid("A Lidarr API key is letters and numbers, from Lidarr's Settings > General.");
    this.hasKey = !!k;
    return this.status(s);
  }

  setHeaders(s: Settings, text: string | null): LidarrStatus {
    this.headers = parseHeaders(text ?? "");
    return this.status(s);
  }

  confirmRoot(s: Settings, path: string, confirmed: boolean) {
    if (!s.lidarrUrl.trim()) throw invalid("Set Lidarr's address first.");
    const key = path.toLowerCase().replace(/[\\/]+$/, "");
    if (confirmed) this.confirmed.add(key);
    else this.confirmed.delete(key);
  }

  check(s: Settings, now: number): LidarrReport {
    if (!s.lidarrUrl.trim() && !s.lidarrAwayUrl.trim()) throw invalid("Set Lidarr's address first.");
    if (!this.hasKey) throw invalid("Add Lidarr's API key first.");
    const unreachable: AppError = { kind: "network", message: "Lidarr isn't reachable. Check the address, and that Lidarr is running." };
    if (this.demo === "unreachable" || this.demo === "offline") throw unreachable;
    if (this.demo === "away" && !s.lidarrAwayUrl.trim()) throw unreachable;
    if (this.demo === "badKey") throw { kind: "auth", message: "Lidarr didn't accept this API key. Copy it again from Lidarr's Settings > General." };
    if (this.demo === "old") {
      throw invalid("This Lidarr is version 1.4.5.3639. Platter needs Lidarr 2.0 or later. Update Lidarr, then check again.");
    }
    const red: LidarrFinding[] = [];
    const amber: LidarrFinding[] = [];
    let good: LidarrFinding[] = [...OK_SETTINGS];
    if (this.demo === "demo") {
      red.push({
        id: "protectedInPlex",
        level: "red",
        title: "Lidarr has a protected artist in your Plex music",
        detail:
          "Quiet Hours (/data/media/Music/Lidarr/Quiet Hours, inside /data/media/Music). Lidarr imports, renames and upgrades in that folder, so curated files could change. Protected artists only ever live in Lidarr's staging folder.",
        fix: ROOT_FIX,
        emptyRoot: null,
        protectedOnly: true,
        action: null,
      });
      amber.push(
        {
          id: "renameTracks",
          level: "amber",
          title: "Lidarr renames the files it downloads",
          detail: "Downloads get Lidarr's own file names instead of keeping theirs.",
          fix: "In Lidarr, turn off Settings > Media Management > Rename Tracks, or let Platter do it.",
          emptyRoot: null,
          protectedOnly: false,
          action: { kind: "renameOff" },
        },
        {
          id: "recycleBin",
          level: "amber",
          title: "Lidarr deletes files for good",
          detail: "Without a recycling bin, files Lidarr replaces or removes can't be brought back.",
          fix: "In Lidarr, set a folder in Settings > Media Management > Recycling Bin.",
          emptyRoot: null,
          protectedOnly: false,
          action: null,
        },
      );
      good = good.filter((f) => f.id !== "renameTracks" && f.id !== "recycleBin");
      good.unshift(ok("artistsInside", "No Lidarr artists in your Plex music folder", "Lidarr doesn't manage any of the folders your library uses."));
    } else if (this.demo === "empty") {
      const confirmed = this.confirmed.has("/lidarr");
      const f: LidarrFinding = confirmed
        ? {
            id: "rootEmpty",
            level: "ok",
            title: "You said /lidarr is a separate folder",
            detail:
              "It's empty, so Platter goes by what you said. Once Lidarr puts music in it, Platter compares its contents with your library before every request.",
            fix: null,
            emptyRoot: { path: "/lidarr", confirmed: true },
            protectedOnly: false,
            action: null,
          }
        : {
            id: "rootEmpty",
            level: "red",
            title: "Lidarr's folder /lidarr is empty",
            detail:
              "An empty folder has nothing to compare, so Platter can't tell whether it's separate from your Plex music folder. If it's a different folder (not your music folder, and not inside it), say so below. Platter checks it again before every request.",
            fix: null,
            emptyRoot: { path: "/lidarr", confirmed: false },
            protectedOnly: false,
            action: null,
          };
      (confirmed ? good : red).unshift(f);
      good.unshift(ok("artistsInside", "No Lidarr artists in your Plex music folder", "Lidarr doesn't manage any of the folders your library uses."));
    } else {
      good.unshift(
        ok("rootSeparate", "Lidarr's folders are separate from your Plex music", "Their contents don't match your library: /downloads/music."),
        ok("artistsInside", "No Lidarr artists in your Plex music folder", "Lidarr doesn't manage any of the folders your library uses."),
      );
    }
    const plain = [s.lidarrUrl, s.lidarrAwayUrl].filter((u) => u && unencrypted(u));
    if (plain.length) {
      amber.push({
        id: "plainHttp",
        level: "amber",
        title: "Your API key would travel unencrypted",
        detail: `${plain.join(", ")} starts with http:// and isn't on your home network, so the key and everything Lidarr answers cross the internet in plain text.`,
        fix: "Reach Lidarr from away through https (your reverse proxy or tunnel can add it), then use the https address here.",
        emptyRoot: null,
        protectedOnly: false,
        action: null,
      });
    }
    const route = this.demo === "away" || !s.lidarrUrl.trim() ? "away" : "home";
    this.lastPassed = red.length === 0;
    const managedOk = !red.some((f) => !f.protectedOnly);
    return {
      version: "2.8.2.4493",
      findings: [...red, ...amber, ...good],
      passed: red.length === 0,
      managedOk,
      protectedCount: this.demo === "demo" ? 1 : 0,
      checkedAt: now,
      route,
    };
  }
}
