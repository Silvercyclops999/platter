/**
 * Listen on in the mock: a few invented Plex players with sessions, and a
 * pretend remote session that ticks like a real player's timeline.
 *
 * Test/demo hooks: `stopElsewhere()` (someone stops the player there),
 * `loseRemote()` (the player stops answering), `setPlayers()`.
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

export function mockPlayers(sample: Track | undefined): RemotePlayer[] {
  return [
    { id: "mock-amp-desk", name: "Studio Desk", product: "Plexamp", kind: "computer", playing: null, online: true },
    {
      id: "mock-sonos",
      name: "Kitchen",
      product: "Sonos",
      kind: "speaker",
      playing: sample ? `${sample.title} • ${sample.trackArtist ?? sample.artistTitle}` : null,
      online: true,
    },
    { id: "mock-phone", name: "Pixel phone", product: "Plexamp", kind: "phone", playing: null, online: true },
    { id: "mock-tv", name: "Living Room TV", product: "Plex for Samsung", kind: "tv", playing: null, online: false },
  ];
}

export class MockRemote {
  private list: RemotePlayer[];
  private session: RemoteSession | null = null;
  private stoppedOn: string | null = null;
  /** The songs the player plays, from the move. */
  private queue: Track[] = [];
  private tick: ReturnType<typeof setInterval> | null = null;

  constructor(
    private host: RemoteHost,
    sample: Track | undefined,
  ) {
    this.list = mockPlayers(sample);
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

  players(): RemotePlayer[] {
    if (!this.host.settings().showPlexPlayers) return [];
    this.host.emitPlayers(this.list);
    return this.list;
  }

  /** Test hook: replace the player list. */
  setPlayers(list: RemotePlayer[]) {
    this.list = list;
  }

  move(id: string): RemoteState {
    const p = this.list.find((x) => x.id === id);
    if (!p) throw { kind: "notFound", message: "That device isn't available any more." };
    if (this.session?.playerId === id) return this.state();
    if (this.session) this.returnHere(false);
    const local = this.host.local();
    if (!local.track) throw { kind: "invalid", message: "Play something first, then move it." };
    if (UNREACHABLE.has(id)) throw { kind: "network", message: `Couldn't reach ${p.name}` };
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
