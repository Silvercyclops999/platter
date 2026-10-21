/**
 * Mock requests ("Ask for music"). With `?asks=guest` you're signed in as
 * someone a server is shared with: Request shows where Get would, and the
 * owner answers after a moment. With `?asks=owner` people you share with
 * have requests waiting for you. Invented people and albums only.
 *
 * Test hooks: `mode`, `canRequest`, `straightAway`, `ownerAnswers(id, yes)`,
 * `arrive(id)`, `incomingArrives(ask)`.
 */
import type { AskAnswered, AskItem, AskState, AsksView, IncomingAsk, LidarrGetOutcome, MyAsk } from "../types";

export type AsksDemo = "guest" | "owner";

export const isAsksDemo = (v: string | null): v is AsksDemo => v === "guest" || v === "owner";

export interface AsksHooks {
  /** The owner's Get, with every check. */
  get: (item: AskItem, releaseGroupId: string) => LidarrGetOutcome;
  emitMine: () => void;
  emitIncoming: (all: IncomingAsk[]) => void;
  notify: (message: string) => void;
  later: (ms: number, fn: () => void) => void;
}

const cover = (rg: string) => `/ext/https://coverartarchive.org/release-group/${rg}/front-250`;
const rg = (n: number) => `c0ffee11-0000-4000-8000-${String(n).padStart(12, "0")}`;
const MIN = 60_000;

const fold = (s: string) => s.toLowerCase().replace(/[^\p{L}\p{N}]+/gu, " ").trim();
const albumOf = (i: AskItem) => (i.song && i.album?.trim() ? i.album : i.title);

/** Same as the core's OPEN_MAX and FULL_NOTE. */
export const OPEN_MAX = 10;
export const FULL_NOTE = "You have 10 requests waiting. Take one back or wait for an answer.";

export class MockAsks {
  mode: AsksDemo | null;
  canRequest: boolean;
  straightAway = false;
  mine: MyAsk[] = [];
  incoming: IncomingAsk[] = [];
  private next = 1;

  constructor(
    mode: AsksDemo | null,
    private hooks: AsksHooks,
  ) {
    this.mode = mode;
    this.canRequest = mode === "guest";
    this.reset(mode, true);
  }

  /** Test hook: start over as the owner, someone it's shared with, or neither, with or without the demo requests. */
  reset(mode: AsksDemo | null, demo = false) {
    this.mode = mode;
    this.canRequest = mode === "guest";
    this.straightAway = false;
    this.incoming = mode === "owner" && demo ? this.demoIncoming() : [];
    this.mine = mode === "guest" && demo ? this.demoMine() : [];
  }

  private demoIncoming(): IncomingAsk[] {
    const now = Date.now();
    const one = (n: number, who: [string, string], title: string, artist: string, ago: number, state: AskState = "requested", extra: Partial<IncomingAsk> = {}): IncomingAsk => ({
      id: `in-${n}`,
      personId: who[0],
      personName: who[1],
      item: { releaseGroupId: rg(n), title, artist, thumb: cover(rg(n)) },
      releaseGroupId: rg(n),
      state,
      progress: null,
      note: null,
      createdAt: now - ago,
      updatedAt: now - ago,
      byItself: false,
      ...extra,
    });
    return [
      one(14, ["71", "robin_listens"], "First Frost", "Cold Coast", 4 * MIN),
      one(21, ["71", "robin_listens"], "Saltmarsh", "Tidewater Hymns", 9 * MIN),
      one(31, ["72", "jules"], "Reedbeds", "Fen Lights", 3 * 60 * MIN),
      one(41, ["72", "jules"], "Copper Seasons", "Marrow & Vine", 26 * 60 * MIN, "getting", { progress: 45 }),
    ];
  }

  private demoMine(): MyAsk[] {
    const now = Date.now();
    const one = (n: number, title: string, artist: string, ago: number, state: AskState, extra: Partial<MyAsk> = {}): MyAsk => ({
      id: `my-${n}`,
      item: { releaseGroupId: rg(n), title, artist, thumb: cover(rg(n)) },
      state,
      progress: null,
      note: null,
      createdAt: now - ago,
      updatedAt: now - ago,
      seen: state !== "sending",
      ...extra,
    });
    return [one(12, "Harbour Static", "Cold Coast", 20 * MIN, "requested"), one(22, "Tidewater Hymns", "Tidewater Hymns", 2 * 24 * 60 * MIN, "getting", { progress: 62 })];
  }

  view(): AsksView {
    if (this.mode !== "guest") return { guest: false, canRequest: false, straightAway: false, mine: [] };
    return { guest: true, canRequest: this.canRequest, straightAway: this.canRequest && this.straightAway, mine: this.mine.filter((a) => a.state !== "cancelled").map((a) => ({ ...a })) };
  }

  request(item: AskItem): MyAsk {
    if (this.mode !== "guest") throw { kind: "invalid", message: "Requests are for music on a server someone shares with you." };
    if (!this.canRequest) throw { kind: "invalid", message: "Requests are turned off for you on this server." };
    const same = this.mine.find(
      (a) =>
        a.state !== "cancelled" &&
        a.state !== "declined" &&
        a.state !== "refused" &&
        a.state !== "failed" &&
        ((item.releaseGroupId && a.item.releaseGroupId === item.releaseGroupId) || (fold(albumOf(a.item)) === fold(albumOf(item)) && fold(a.item.artist) === fold(item.artist))),
    );
    if (same) return { ...same };
    if (this.mine.filter((a) => a.state === "sending" || a.state === "requested").length >= OPEN_MAX) throw { kind: "invalid", message: FULL_NOTE };
    const now = Date.now();
    const a: MyAsk = { id: `my-new-${this.next++}`, item: { ...item }, state: "sending", progress: null, note: null, createdAt: now, updatedAt: now, seen: false };
    this.mine.unshift(a);
    this.hooks.emitMine();
    // The owner's Platter picks it up.
    this.hooks.later(600, () => {
      if (a.state !== "sending") return;
      a.state = this.straightAway ? "getting" : "requested";
      a.seen = true;
      this.hooks.emitMine();
    });
    return { ...a };
  }

  cancel(id: string) {
    const a = this.mine.find((x) => x.id === id);
    if (!a) return;
    if (a.state === "getting" || a.state === "ready") throw { kind: "invalid", message: "It's already on its way, so it can't be taken back." };
    a.state = "cancelled";
    this.hooks.emitMine();
  }

  /** Test hook: the owner answered one of my requests. */
  ownerAnswers(id: string, yes: boolean) {
    const a = this.mine.find((x) => x.id === id);
    if (!a) return;
    a.state = yes ? "getting" : "declined";
    a.seen = true;
    this.hooks.emitMine();
  }

  /** Test hook: one of my requests is on the server now. */
  arrive(id: string) {
    const a = this.mine.find((x) => x.id === id);
    if (!a) return;
    a.state = "ready";
    a.progress = null;
    this.hooks.emitMine();
    this.hooks.notify(`${albumOf(a.item)} is here.`);
  }

  incomingList(): IncomingAsk[] {
    return this.incoming
      .filter((a) => a.state !== "cancelled")
      .sort((a, b) => b.createdAt - a.createdAt)
      .map((a) => ({ ...a, item: { ...a.item } }));
  }

  /** Test hook: someone's request reached this (the owner's) Platter. */
  incomingArrives(a: IncomingAsk) {
    this.incoming.push(a);
    this.hooks.emitIncoming(this.incomingList());
  }

  answer(id: string, yes: boolean): AskAnswered {
    if (this.mode !== "owner") throw { kind: "invalid", message: "Only the server's owner answers requests." };
    const a = this.incoming.find((x) => x.id === id);
    if (!a) throw { kind: "notFound", message: "That request is gone." };
    let needs: AskAnswered["needs"] = null;
    if (a.state === "requested") {
      if (!yes) {
        a.state = "declined";
      } else if (a.releaseGroupId) {
        const out = this.hooks.get(a.item, a.releaseGroupId);
        if (out.record) {
          a.state = out.record.state === "imported" ? "ready" : "getting";
          a.progress = out.record.progress ?? null;
        } else if (out.needs?.kind === "firstTime") {
          needs = out.needs;
        } else if (out.needs) {
          needs = out.needs;
          a.state = "refused";
          a.note = out.needs.reason;
        }
      }
      a.updatedAt = Date.now();
    }
    this.hooks.emitIncoming(this.incomingList());
    return { incoming: { ...a }, needs };
  }
}
