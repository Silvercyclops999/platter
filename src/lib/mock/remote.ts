/**
 * Listen on in the mock: another Platter computer of the same account, a
 * few invented Plex players with sessions, a Chromecast, a Cast speaker
 * group and AirPlay speakers (one with a password, one Platter can't play
 * on), and a pretend remote session that ticks like a real player's
 * timeline.
 *
 * Test/demo hooks: `stopElsewhere()` (someone stops the player there),
 * `loseRemote()` (the player stops answering), `setPlayers()`,
 * `setPlatterDevices()`. `?platter=unreachable` in the URL shows the other
 * Platter as seen through Plex but not answering (the firewall note).
 */
import type { PlayerState, RemoteCommand, RemotePlayer, RemoteSession, RemoteState, Settings, Track } from "../types";

export interface RemoteHost {
  local(): PlayerState;
  pauseLocal(): void;
  /** Put local playback on this song at this spot. */
  resumeAt(trackId: string, ms: number, play: boolean): void;
  emitState(s: RemoteState): void;
  emitPlayers(p: RemotePlayer[]): void;
  toast(message: string): void;
  settings(): Settings;
  later(ms: number, fn: () => void): void;
}

/** Ids of players that refuse commands (to demo "Couldn't reach"). */
export const UNREACHABLE = new Set(["mock-tv"]);

/** The mock AirPlay speaker's password. */
export const MOCK_AIRPLAY_PASSWORD = "1234";
export const NEEDS_PASSWORD = "This speaker needs its AirPlay password.";
export const WRONG_PASSWORD = "That AirPlay password didn't work.";

const plex = { via: "plex" as const, needsPassword: false, unsupported: null, unreachable: false };
const lan = (via: "cast" | "airplay") => ({ via, needsPassword: false, unsupported: null, playing: null, online: true, unreachable: false });

/** Other Platter computers signed in to the same account. */
export function mockPlatterDevices(unreachable = false): RemotePlayer[] {
  return [
    {
      id: "mock-platter-mac",
      name: "Studio MacBook",
      product: "Platter",
      kind: "computer",
      playing: null,
      online: true,
      via: "platter",
      needsPassword: false,
      unsupported: null,
      unreachable,
    },
  ];
}

/** Cast devices and AirPlay receivers "on the network". */
export function mockLanDevices(): RemotePlayer[] {
  return [
    { id: "cast:den", name: "Den TV", product: "Chromecast", kind: "tv", ...lan("cast") },
    { id: "cast:group", name: "Downstairs", product: "Speaker group", kind: "group", ...lan("cast") },
    { id: "airplay:patio", name: "Patio", product: "AirPlay", kind: "speaker", ...lan("airplay"), needsPassword: true },
    {
      id: "airplay:old",
      name: "Old AirPort",
      product: "AirPlay",
      kind: "speaker",
      ...lan("airplay"),
      unsupported: "Needs AirPlay encryption Platter doesn't support",
    },
  ];
}

export function mockPlayers(sample: Track | undefined): RemotePlayer[] {
  return [
    { id: "mock-amp-desk", name: "Studio Desk", product: "Plexamp", kind: "computer", playing: null, online: true, ...plex },
    {
      id: "mock-sonos",
      name: "Kitchen",
      product: "Sonos",
      kind: "speaker",
      playing: sample ? `${sample.title} • ${sample.trackArtist ?? sample.artistTitle}` : null,
      online: true,
      ...plex,
    },
    { id: "mock-phone", name: "Pixel phone", product: "Plexamp", kind: "phone", playing: null, online: true, ...plex },
    { id: "mock-tv", name: "Living Room TV", product: "Plex for Samsung", kind: "tv", playing: null, online: false, ...plex },
  ];
}

export class MockRemote {
  private list: RemotePlayer[];
  private lanList: RemotePlayer[] = mockLanDevices();
  private platterList: RemotePlayer[];
  /** AirPlay passwords "in the keychain". */
  private passwords = new Map<string, string>();
  private session: RemoteSession | null = null;
  private stoppedOn: string | null = null;
  /** The songs the player plays, from the move. */
  private queue: Track[] = [];
  private tick: ReturnType<typeof setInterval> | null = null;

  constructor(
    private host: RemoteHost,
    sample: Track | undefined,
    platterUnreachable = false,
  ) {
    this.list = mockPlayers(sample);
    this.platterList = mockPlatterDevices(platterUnreachable);
  }

  dispose() {
    this.stopTick();
  }

  state(): RemoteState {
    return { session: this.session ? { ...this.session } : null, stoppedOn: this.stoppedOn };
  }

  private emit() {
    this.host.emitState(this.state());
  }

  private all(): RemotePlayer[] {
    const s = this.host.settings();
    const lan = this.lanList
      .map((p) => ({ ...p, needsPassword: p.needsPassword && !this.passwords.has(p.id) }))
      // Receivers that ask for a password are hidden unless the setting shows them.
      .filter((p) => s.showAirplayLocked || !p.needsPassword);
    return [...this.platterList, ...(s.showPlexPlayers ? this.list : []), ...(s.showCastDevices ? lan : [])];
  }

  players(): RemotePlayer[] {
    const list = this.all();
    this.host.emitPlayers(list);
    return list;
  }

  /** Test hook: replace the Plex player list. */
  setPlayers(list: RemotePlayer[]) {
    this.list = list;
  }

  /** Test hook: replace the other Platter computers. */
  setPlatterDevices(list: RemotePlayer[]) {
    this.platterList = list;
  }

  /** Test hook: replace the Cast and AirPlay devices. */
  setLanDevices(list: RemotePlayer[]) {
    this.lanList = list;
  }

  /** Test hook: the starting Cast and AirPlay devices, no passwords kept. */
  resetLan() {
    this.lanList = mockLanDevices();
    this.passwords.clear();
  }

  /** Move to an AirPlay speaker with its password. */
  moveWithPassword(id: string, password: string): RemoteState {
    if (id === "airplay:patio" && password !== MOCK_AIRPLAY_PASSWORD) throw { kind: "auth", message: WRONG_PASSWORD };
    const st = this.move(id, password);
    this.passwords.set(id, password);
    return st;
  }

  move(id: string, password?: string): RemoteState {
    const p = this.all().find((x) => x.id === id) ?? this.list.find((x) => x.id === id);
    if (!p) throw { kind: "notFound", message: "That device isn't available any more." };
    if (p.unsupported) throw { kind: "invalid", message: `Can't play on ${p.name}: ${p.unsupported}.` };
    if (id === "airplay:patio" && (password ?? this.passwords.get(id)) !== MOCK_AIRPLAY_PASSWORD) {
      throw { kind: "auth", message: NEEDS_PASSWORD };
    }
    if (this.session?.playerId === id) return this.state();
    if (this.session) this.returnHere(false);
    const local = this.host.local();
    if (!local.track) throw { kind: "invalid", message: "Play something first, then move it." };
    if (UNREACHABLE.has(id) || p.unreachable) throw { kind: "network", message: `Couldn't reach ${p.name}` };
    this.queue = [local.track, ...local.queue.userQueue.map((i) => i.track), ...local.queue.nextUp.map((i) => i.track)];
    if (this.host.settings().moveStopsOldDevice) this.host.pauseLocal();
    this.stoppedOn = null;
    this.session = {
      playerId: p.id,
      playerName: p.name,
      kind: p.kind,
      status: "connecting",
      track: local.track,
      positionMs: local.positionMs,
      durationMs: local.track.durationMs,
      volume: 0.6,
      shuffle: local.shuffle,
      repeat: local.repeat,
    };
    this.host.later(600, () => {
      if (this.session?.playerId !== id || this.session.status !== "connecting") return;
      this.session.status = "playing";
      this.emit();
    });
    this.startTick();
    this.emit();
    return this.state();
  }

  /** Send the (new) local queue to the device playing now. */
  resend(): RemoteState {
    const s = this.session;
    if (!s) throw { kind: "invalid", message: "Nothing is playing on another device." };
    this.session = null;
    return this.move(s.playerId);
  }

  returnHere(emit = true): RemoteState {
    const s = this.session;
    this.stopTick();
    this.session = null;
    this.stoppedOn = null;
    if (s?.track) this.host.resumeAt(s.track.id, s.positionMs, s.status !== "paused");
    if (emit) this.emit();
    return this.state();
  }

  command(c: RemoteCommand): RemoteState {
    const s = this.session;
    if (!s) throw { kind: "invalid", message: "Nothing is playing on another device." };
    switch (c.kind) {
      case "play":
        s.status = "playing";
        break;
      case "pause":
        s.status = "paused";
        break;
      case "next":
        this.step(1);
        break;
      case "previous":
        if (s.positionMs >= 3000) s.positionMs = 0;
        else this.step(-1);
        break;
      case "seek":
        s.positionMs = Math.max(0, Math.min(c.positionMs, s.durationMs));
        break;
      case "volume":
        s.volume = Math.max(0, Math.min(1, c.volume));
        break;
      case "shuffle":
        s.shuffle = c.on;
        break;
      case "repeat":
        s.repeat = c.mode;
        break;
    }
    this.emit();
    return this.state();
  }

  /** Test/demo hook: someone stopped the player there. */
  stopElsewhere() {
    const s = this.session;
    if (!s) return;
    this.stopTick();
    this.session = null;
    this.stoppedOn = s.playerName;
    if (s.track) this.host.resumeAt(s.track.id, s.positionMs, false);
    this.emit();
  }

  /** Test/demo hook: the player stopped answering. */
  loseRemote() {
    const s = this.session;
    if (!s) return;
    this.stopTick();
    this.session = null;
    this.host.toast(`Couldn't reach ${s.playerName}`);
    if (s.track) this.host.resumeAt(s.track.id, s.positionMs, false);
    this.emit();
  }

  private step(by: number) {
    const s = this.session;
    if (!s?.track) return;
    const i = this.queue.findIndex((t) => t.id === s.track!.id);
    const next = this.queue[i + by];
    if (!next) {
      s.positionMs = 0;
      return;
    }
    s.track = next;
    s.positionMs = 0;
    s.durationMs = next.durationMs;
  }

  private startTick() {
    this.stopTick();
    this.tick = setInterval(() => {
      const s = this.session;
      if (!s || s.status !== "playing") return;
      s.positionMs += 1000;
      if (s.positionMs >= s.durationMs) this.step(1);
      this.emit();
    }, 1000);
  }

  private stopTick() {
    if (this.tick) clearInterval(this.tick);
    this.tick = null;
  }
}
