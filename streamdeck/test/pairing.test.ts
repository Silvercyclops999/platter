import { afterEach, describe, expect, it, vi } from "vitest";
import { PlatterClient, PlatterError, type Request, type Response, StatePoller } from "../src/client.js";
import { AutoPair } from "../src/pairing.js";
import { dialImage, newDial } from "../src/dials.js";
import { Live } from "../src/live.js";
import { svgOf } from "../src/render.js";
import { settingsFor } from "../src/settings.js";
import { APPROVE, keyView } from "../src/views.js";

/** A fake Platter: answers pairing requests the way the real one does. */
function platter() {
  const sent: Request[] = [];
  const p = {
    sent,
    up: true,
    enabled: true,
    /** The open prompt's id. */
    open: null as string | null,
    answer: null as null | "allowed" | "denied",
    n: 0,
    async send(r: Request): Promise<Response> {
      sent.push(r);
      if (!p.up) throw new Error("ECONNREFUSED");
      if (r.path === "/v1/pair/request") {
        if (!p.enabled) return { status: 403, body: { error: "Turn on Allow Stream Deck in Platter's Settings first." } };
        if (p.open) return { status: 409, body: { error: "Platter is already asking. Answer it in Platter's window." } };
        p.open = `req${++p.n}`;
        p.answer = null;
        return { status: 200, body: { request: p.open } };
      }
      if (r.path === "/v1/pair/status") {
        const id = (r.body as { request: string }).request;
        if (id !== p.open) return { status: 200, body: { state: "gone" } };
        if (p.answer === "allowed") {
          p.open = null;
          return { status: 200, body: { state: "allowed", token: "tok" } };
        }
        if (p.answer === "denied") return { status: 200, body: { state: "denied" } };
        return { status: 200, body: { state: "waiting" } };
      }
      if (r.path.startsWith("/v1/state")) {
        if (r.token !== "tok") return { status: 401, body: { error: "Pair with Platter first." } };
        return {
          status: 200,
          body: { api: 1, playing: true, private: false, title: "Night Drive", artist: "Rue Ellis", album: "Late", trackId: "t1", liked: false, volume: 50, muted: false, coverKey: null, canRequest: false },
        };
      }
      return { status: 404, body: { error: "Not found." } };
    },
  };
  return p;
}

function rig(token: string | null = null) {
  const p = platter();
  const client = new PlatterClient((r) => p.send(r), token);
  const kept: { token: string | null; request: string | null } = { token: null, request: null };
  const pairer: AutoPair = new AutoPair(client, {
    onToken: (t) => void (kept.token = t),
    onRequest: (r) => void (kept.request = r),
    onChange: () => void poller.refresh(),
  });
  const poller = new StatePoller(client, 1000, 5000, pairer);
  // What the keys show: the connection state, with no grace period for Platter missing.
  const live = new Live(Date.now, 0);
  const titles: string[] = [];
  const stop = poller.subscribe((s, err) => titles.push(live.update(s, err).conn));
  return { p, client, pairer, poller, kept, titles, stop, asks: () => p.sent.filter((r) => r.path === "/v1/pair/request").length };
}

describe("pairing on first connect", () => {
  afterEach(() => vi.useRealTimers());

  it("asks Platter, says Approve in Platter, and works once allowed", async () => {
    vi.useFakeTimers();
    const r = rig();
    r.pairer.start();
    await vi.advanceTimersByTimeAsync(0);
    expect(r.asks()).toBe(1);
    expect(r.p.sent[0]).toEqual({ method: "POST", path: "/v1/pair/request", body: { client: "streamdeck" } });
    expect(r.titles.at(-1)).toBe("approve");
    expect(r.kept.request).toBe("req1");
    // Waiting: checks for an answer, never asks twice.
    await vi.advanceTimersByTimeAsync(10_000);
    expect(r.asks()).toBe(1);
    expect(r.client.paired).toBe(false);
    expect(r.titles.at(-1)).toBe("approve");
    r.p.answer = "allowed";
    await vi.advanceTimersByTimeAsync(1000);
    expect(r.client.paired).toBe(true);
    expect(r.kept).toEqual({ token: "tok", request: null });
    expect(r.titles.at(-1)).toBe("ok");
    // Every key uses that one pairing.
    const states = r.p.sent.filter((x) => x.path.startsWith("/v1/state"));
    expect(states.every((x) => x.token === "tok")).toBe(true);
    r.stop();
    r.pairer.stop();
  });

  it("doesn't ask when it already has a token", async () => {
    vi.useFakeTimers();
    const r = rig("tok");
    r.pairer.start();
    await vi.advanceTimersByTimeAsync(20_000);
    expect(r.asks()).toBe(0);
    expect(r.titles.at(-1)).toBe("ok");
    r.stop();
  });

  it("after Don't allow it waits for a key press to ask again", async () => {
    vi.useFakeTimers();
    const r = rig();
    r.pairer.start();
    await vi.advanceTimersByTimeAsync(0);
    r.p.answer = "denied";
    await vi.advanceTimersByTimeAsync(1000);
    expect(r.pairer.phase).toBe("denied");
    expect(r.titles.at(-1)).toBe("unpaired");
    expect(r.client.paired).toBe(false);
    // Polling never asks again by itself.
    r.p.open = null;
    await vi.advanceTimersByTimeAsync(60_000);
    expect(r.asks()).toBe(1);
    r.pairer.press();
    await vi.advanceTimersByTimeAsync(0);
    expect(r.asks()).toBe(2);
    expect(r.titles.at(-1)).toBe("approve");
    r.stop();
    r.pairer.stop();
  });

  it("keeps trying while Platter isn't running, then asks once it is", async () => {
    vi.useFakeTimers();
    const r = rig();
    r.p.up = false;
    r.pairer.start();
    await vi.advanceTimersByTimeAsync(0);
    expect(r.titles.at(-1)).toBe("offline");
    await vi.advanceTimersByTimeAsync(5000);
    expect(r.asks()).toBeGreaterThanOrEqual(2);
    r.p.up = true;
    await vi.advanceTimersByTimeAsync(5000);
    expect(r.pairer.phase).toBe("waiting");
    expect(r.p.open).toBe("req1");
    r.stop();
    r.pairer.stop();
  });

  it("with Allow Stream Deck off it keeps trying quietly, and asks once it's on", async () => {
    vi.useFakeTimers();
    const r = rig();
    r.p.enabled = false;
    r.pairer.start();
    await vi.advanceTimersByTimeAsync(0);
    expect(r.pairer.phase).toBe("idle");
    r.p.enabled = true;
    await vi.advanceTimersByTimeAsync(5000);
    expect(r.pairer.phase).toBe("waiting");
    r.stop();
    r.pairer.stop();
  });

  it("while Platter is already asking, it waits and asks again later", async () => {
    vi.useFakeTimers();
    const r = rig();
    r.p.open = "someone-else";
    r.pairer.start();
    await vi.advanceTimersByTimeAsync(0);
    expect(r.pairer.phase).toBe("asking");
    expect(r.titles.at(-1)).toBe("approve");
    r.p.open = null;
    await vi.advanceTimersByTimeAsync(5000);
    expect(r.pairer.phase).toBe("waiting");
    r.stop();
    r.pairer.stop();
  });

  it("picks up a request from before a restart", async () => {
    vi.useFakeTimers();
    const r = rig();
    r.p.open = "req9";
    r.pairer.start("req9");
    await vi.advanceTimersByTimeAsync(0);
    expect(r.asks()).toBe(0);
    expect(r.pairer.phase).toBe("waiting");
    r.p.answer = "allowed";
    await vi.advanceTimersByTimeAsync(1000);
    expect(r.kept.token).toBe("tok");
    r.stop();
  });

  it("an unanswered prompt that went away stops asking until a key is pressed", async () => {
    vi.useFakeTimers();
    const r = rig();
    r.pairer.start();
    await vi.advanceTimersByTimeAsync(0);
    r.p.open = null; // timed out in Platter
    await vi.advanceTimersByTimeAsync(1000);
    expect(r.pairer.phase).toBe("idle");
    await vi.advanceTimersByTimeAsync(60_000);
    expect(r.asks()).toBe(1);
    expect(r.titles.at(-1)).toBe("unpaired");
    r.stop();
  });

  it("keys say Approve in Platter, the dial on one line; Don't allow shows the not-paired badge", () => {
    const live = new Live(Date.now, 0);
    const approve = live.update(null, new PlatterError("approve", "x"));
    expect(approve.conn).toBe("approve");
    expect(keyView("next", {}, { snap: approve, now: 0, cover: () => null, item: () => undefined }).title).toBe(APPROVE);
    const dial = svgOf(dialImage("volume", settingsFor("volume", {}) as unknown as Record<string, unknown>, newDial(), approve, 0, { browse: [], queue: [], devices: [] }, () => null));
    expect(dial).toContain("Approve in Platter");
    expect(live.update(null, new PlatterError("denied", "x")).conn).toBe("unpaired");
  });
});
