import * as http from "node:http";
import type { AddressInfo } from "node:net";
import { afterEach, describe, expect, it, vi } from "vitest";
import { httpTransport, PlatterClient, PlatterError, type PlatterState, type Request, type Response, StatePoller, toError } from "../src/client.js";
import { likeView, playPauseView, plainView, volumeFeedback } from "../src/view.js";

function state(over: Partial<PlatterState> = {}): PlatterState {
  return {
    api: 1,
    playing: true,
    private: false,
    title: "Night Drive",
    artist: "Rue Ellis",
    album: "Late",
    trackId: "t1",
    liked: false,
    volume: 50,
    muted: false,
    coverKey: "k1",
    cover: "data:image/jpeg;base64,AAA",
    canRequest: true,
    ...over,
  };
}

/** A transport that answers from a script and records what was sent. */
function fake(answer: (r: Request) => Response | Promise<Response>) {
  const sent: Request[] = [];
  const t = async (r: Request) => {
    sent.push(r);
    return answer(r);
  };
  return { t, sent };
}

describe("PlatterClient", () => {
  it("pairs with the six digits and keeps the token", async () => {
    const { t, sent } = fake(() => ({ status: 200, body: { token: "tok" } }));
    const c = new PlatterClient(t);
    expect(c.paired).toBe(false);
    expect(await c.pair(" 123 456 ")).toBe("tok");
    expect(c.paired).toBe(true);
    expect(sent[0]).toEqual({ method: "POST", path: "/v1/pair", body: { code: "123456" } });
  });

  it("refuses a code that isn't six digits without asking", async () => {
    const { t, sent } = fake(() => ({ status: 200, body: {} }));
    await expect(new PlatterClient(t).pair("12")).rejects.toMatchObject({ kind: "wrongCode" });
    expect(sent).toHaveLength(0);
  });

  it("a wrong code says so", async () => {
    const { t } = fake(() => ({ status: 401, body: { error: "That code doesn't match." } }));
    await expect(new PlatterClient(t).pair("000000")).rejects.toMatchObject({ kind: "wrongCode", message: "That code doesn't match." });
  });

  it("sends the token with state and commands", async () => {
    const { t, sent } = fake((r) => (r.path.startsWith("/v1/state") ? { status: 200, body: state() } : { status: 200, body: { ok: true } }));
    const c = new PlatterClient(t, "tok");
    await c.state();
    await c.command({ command: "volumeStep", value: -4 });
    expect(sent.map((r) => r.token)).toEqual(["tok", "tok"]);
    expect(sent[1]).toMatchObject({ method: "POST", path: "/v1/command", body: { command: "volumeStep", value: -4 } });
  });

  it("asks for the cover only when it changes, and keeps it between polls", async () => {
    let n = 0;
    const { t, sent } = fake(() => {
      n++;
      if (n === 1) return { status: 200, body: state() };
      if (n === 2) return { status: 200, body: state({ cover: undefined }) };
      if (n === 3) return { status: 200, body: state({ coverKey: "k2", cover: "data:image/jpeg;base64,BBB" }) };
      return { status: 200, body: state({ coverKey: null, cover: undefined }) };
    });
    const c = new PlatterClient(t, "tok");
    expect((await c.state()).cover).toBe("data:image/jpeg;base64,AAA");
    expect((await c.state()).cover).toBe("data:image/jpeg;base64,AAA");
    expect((await c.state()).cover).toBe("data:image/jpeg;base64,BBB");
    expect((await c.state()).cover).toBeNull();
    expect(sent.map((r) => r.path)).toEqual(["/v1/state", "/v1/state?cover=k1", "/v1/state?cover=k1", "/v1/state?cover=k2"]);
  });

  it("without a token it doesn't ask at all", async () => {
    const { t, sent } = fake(() => ({ status: 200, body: {} }));
    const c = new PlatterClient(t);
    await expect(c.state()).rejects.toMatchObject({ kind: "unpaired" });
    await expect(c.command({ command: "next" })).rejects.toMatchObject({ kind: "unpaired" });
    expect(sent).toHaveLength(0);
  });

  it("Platter not running is offline", async () => {
    const c = new PlatterClient(() => Promise.reject(new Error("ECONNREFUSED")), "tok");
    await expect(c.state()).rejects.toMatchObject({ kind: "offline" });
    await expect(c.hello()).rejects.toBeInstanceOf(PlatterError);
  });

  it("maps answers to error kinds with Platter's words", () => {
    expect(toError({ status: 401, body: {} }).kind).toBe("unpaired");
    expect(toError({ status: 429, body: {} }).kind).toBe("busy");
    expect(toError({ status: 403, body: {} }).kind).toBe("refused");
    const e = toError({ status: 409, body: { error: "Set up Lidarr in Platter's Settings first." } });
    expect(e.kind).toBe("failed");
    expect(e.message).toBe("Set up Lidarr in Platter's Settings first.");
  });
});

describe("httpTransport", () => {
  let server: http.Server | null = null;
  afterEach(() => {
    server?.close();
    server = null;
  });

  it("talks to 127.0.0.1 with no browser headers", async () => {
    let seen: http.IncomingHttpHeaders = {};
    let body = "";
    server = http.createServer((req, res) => {
      seen = req.headers;
      req.on("data", (c) => (body += c));
      req.on("end", () => {
        res.writeHead(200, { "Content-Type": "application/json" });
        res.end(JSON.stringify({ ok: true }));
      });
    });
    await new Promise<void>((r) => server!.listen(0, "127.0.0.1", r));
    const port = (server.address() as AddressInfo).port;
    const res = await httpTransport(port)({ method: "POST", path: "/v1/command", token: "tok", body: { command: "next" } });
    expect(res).toEqual({ status: 200, body: { ok: true } });
    expect(seen.host).toBe(`127.0.0.1:${port}`);
    expect(seen.authorization).toBe("Bearer tok");
    expect(seen.origin).toBeUndefined();
    expect(seen["sec-fetch-site"]).toBeUndefined();
    expect(JSON.parse(body)).toEqual({ command: "next" });
  });

  it("rejects when nothing listens", async () => {
    const probe = http.createServer();
    await new Promise<void>((r) => probe.listen(0, "127.0.0.1", r));
    const port = (probe.address() as AddressInfo).port;
    await new Promise((r) => probe.close(r));
    await expect(httpTransport(port, 500)({ method: "GET", path: "/v1/hello" })).rejects.toBeTruthy();
  });
});

describe("StatePoller", () => {
  afterEach(() => vi.useRealTimers());

  it("polls every second while Platter answers and slows down when it doesn't", async () => {
    vi.useFakeTimers();
    let up = true;
    const { t, sent } = fake(() => {
      if (!up) throw new Error("down");
      return { status: 200, body: state() };
    });
    const p = new StatePoller(new PlatterClient(t, "tok"));
    const seen: (string | null)[] = [];
    const stop = p.subscribe((s, err) => seen.push(s ? "state" : err!.kind));
    await vi.advanceTimersByTimeAsync(0);
    await vi.advanceTimersByTimeAsync(2000);
    expect(sent.length).toBe(3);
    up = false;
    await vi.advanceTimersByTimeAsync(1000);
    expect(seen.at(-1)).toBe("offline");
    const before = sent.length;
    await vi.advanceTimersByTimeAsync(4000);
    expect(sent.length, "waits five seconds after a failure").toBe(before);
    await vi.advanceTimersByTimeAsync(1000);
    expect(sent.length).toBe(before + 1);
    stop();
    const after = sent.length;
    await vi.advanceTimersByTimeAsync(10000);
    expect(sent.length, "nothing polls with no keys showing").toBe(after);
  });

  it("unpaired keys say so without asking Platter", async () => {
    vi.useFakeTimers();
    const { t, sent } = fake(() => ({ status: 200, body: state() }));
    const p = new StatePoller(new PlatterClient(t));
    const errs: string[] = [];
    const stop = p.subscribe((_, err) => err && errs.push(err.kind));
    await vi.advanceTimersByTimeAsync(0);
    expect(errs).toEqual(["unpaired"]);
    expect(sent).toHaveLength(0);
    stop();
  });
});

describe("key views", () => {
  const off = new PlatterError("offline", "x");
  const unpaired = new PlatterError("unpaired", "x");

  it("play / pause shows the cover, or the play and pause images", () => {
    expect(playPauseView(state(), null)).toEqual({ state: 1, image: "data:image/jpeg;base64,AAA" });
    expect(playPauseView(state({ playing: false, cover: null }), null)).toEqual({ state: 0, image: undefined });
    expect(playPauseView(null, unpaired)).toEqual({ state: 0, title: "Pair" });
    expect(playPauseView(null, off).title).toBe("Off");
  });

  it("like follows the song", () => {
    expect(likeView(state({ liked: true }), null).state).toBe(1);
    expect(likeView(state(), null).state).toBe(0);
    expect(plainView(state(), null).title).toBeUndefined();
  });

  it("the dial shows the volume, or muted", () => {
    expect(volumeFeedback(state({ volume: 42 }), null)).toEqual({ title: "Volume", value: "42%", indicator: 42 });
    expect(volumeFeedback(state({ muted: true }), null)).toEqual({ title: "Volume", value: "Muted", indicator: 0 });
    expect(volumeFeedback(null, off).value).toBe("Off");
  });
});
