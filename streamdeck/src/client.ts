/**
 * Talks to Platter's local control API on 127.0.0.1. Plain node:http, so no
 * browser headers (Platter turns those away). No Stream Deck code in here,
 * so it can be tested on its own.
 */
import * as http from "node:http";

export const DEFAULT_PORT = 32541;

/** What Platter says is playing (GET /v1/state). */
export interface PlatterState {
  api: number;
  playing: boolean;
  private: boolean;
  title: string | null;
  artist: string | null;
  album: string | null;
  trackId: string | null;
  liked: boolean;
  /** 0 to 100. */
  volume: number;
  muted: boolean;
  coverKey: string | null;
  /** A data image, kept by the client between polls. */
  cover?: string | null;
  canRequest: boolean;
}

export type Command =
  | { command: "playPause" }
  | { command: "next" }
  | { command: "previous" }
  | { command: "like" }
  | { command: "volumeSet"; value: number }
  | { command: "volumeStep"; value: number }
  | { command: "requestAlbum" }
  | { command: "listenAlong" };

export interface Request {
  method: "GET" | "POST";
  path: string;
  token?: string;
  body?: unknown;
}

export interface Response {
  status: number;
  body: unknown;
}

/** Sends one request. Rejects when Platter can't be reached. */
export type Transport = (req: Request) => Promise<Response>;

/** Why a call failed. */
export type ErrorKind = "offline" | "unpaired" | "wrongCode" | "busy" | "refused" | "failed";

export class PlatterError extends Error {
  constructor(
    readonly kind: ErrorKind,
    message: string,
  ) {
    super(message);
    this.name = "PlatterError";
  }
}

/** The real transport: HTTP to 127.0.0.1 only. */
export function httpTransport(port = DEFAULT_PORT, timeoutMs = 4000): Transport {
  return (req) =>
    new Promise((resolve, reject) => {
      const payload = req.body === undefined ? undefined : Buffer.from(JSON.stringify(req.body));
      const headers: Record<string, string> = { Host: `127.0.0.1:${port}`, Accept: "application/json" };
      if (req.token) headers.Authorization = `Bearer ${req.token}`;
      if (payload) {
        headers["Content-Type"] = "application/json";
        headers["Content-Length"] = String(payload.length);
      }
      const r = http.request({ host: "127.0.0.1", port, method: req.method, path: req.path, headers, timeout: timeoutMs }, (res) => {
        const chunks: Buffer[] = [];
        res.on("data", (c: Buffer) => chunks.push(c));
        res.on("end", () => {
          let body: unknown = null;
          try {
            body = JSON.parse(Buffer.concat(chunks).toString("utf8"));
          } catch {
            body = null;
          }
          resolve({ status: res.statusCode ?? 0, body });
        });
      });
      r.on("timeout", () => r.destroy(new Error("timed out")));
      r.on("error", reject);
      if (payload) r.write(payload);
      r.end();
    });
}

function errorText(body: unknown): string {
  return body && typeof body === "object" && "error" in body && typeof body.error === "string" ? body.error : "";
}

/** Turn an answer that isn't 200 into a PlatterError. */
export function toError(res: Response): PlatterError {
  const text = errorText(res.body);
  switch (res.status) {
    case 401:
      return new PlatterError("unpaired", text || "Pair with Platter first.");
    case 429:
      return new PlatterError("busy", text || "Too many requests.");
    case 403:
      return new PlatterError("refused", text || "Platter refused the request.");
    default:
      return new PlatterError("failed", text || `Platter answered ${res.status}.`);
  }
}

export class PlatterClient {
  private cover: { key: string; data: string } | null = null;

  constructor(
    private readonly send: Transport,
    private token: string | null = null,
  ) {}

  setToken(token: string | null): void {
    this.token = token;
  }

  get paired(): boolean {
    return !!this.token;
  }

  private async call(req: Request): Promise<unknown> {
    let res: Response;
    try {
      res = await this.send(req);
    } catch {
      throw new PlatterError("offline", "Platter isn't running, or Allow Stream Deck is off in its Settings.");
    }
    if (res.status !== 200) throw toError(res);
    return res.body;
  }

  /** Whether Platter answers, and whether our token is paired. */
  async hello(): Promise<{ paired: boolean }> {
    const b = (await this.call({ method: "GET", path: "/v1/hello", token: this.token ?? undefined })) as { paired?: boolean };
    return { paired: !!b?.paired };
  }

  /** Swap the code from Platter's Settings for a token (kept by the caller). */
  async pair(code: string): Promise<string> {
    const clean = code.replace(/\D/g, "");
    if (clean.length !== 6) throw new PlatterError("wrongCode", "The code is the six digits shown in Platter's Settings.");
    try {
      const b = (await this.call({ method: "POST", path: "/v1/pair", body: { code: clean } })) as { token?: string };
      if (!b?.token) throw new PlatterError("failed", "Platter didn't send a token.");
      this.token = b.token;
      return b.token;
    } catch (e) {
      if (e instanceof PlatterError && e.kind === "unpaired") throw new PlatterError("wrongCode", e.message);
      throw e;
    }
  }

  /** What's playing. The cover is only fetched when it changes. */
  async state(): Promise<PlatterState> {
    if (!this.token) throw new PlatterError("unpaired", "Pair with Platter first.");
    const have = this.cover ? `?cover=${encodeURIComponent(this.cover.key)}` : "";
    const s = (await this.call({ method: "GET", path: `/v1/state${have}`, token: this.token })) as PlatterState;
    if (!s.coverKey) {
      this.cover = null;
      s.cover = null;
    } else if (s.cover) {
      this.cover = { key: s.coverKey, data: s.cover };
    } else {
      s.cover = this.cover?.key === s.coverKey ? this.cover.data : null;
    }
    return s;
  }

  async command(cmd: Command): Promise<void> {
    if (!this.token) throw new PlatterError("unpaired", "Pair with Platter first.");
    await this.call({ method: "POST", path: "/v1/command", token: this.token, body: cmd });
  }
}

type Listener = (s: PlatterState | null, err: PlatterError | null) => void;

/**
 * Polls the state while anyone listens: every second while Platter answers,
 * every five seconds while it doesn't, not at all while unpaired.
 */
export class StatePoller {
  private listeners = new Set<Listener>();
  private timer: ReturnType<typeof setTimeout> | null = null;
  private running = false;
  last: PlatterState | null = null;

  constructor(
    private readonly client: PlatterClient,
    readonly fastMs = 1000,
    readonly slowMs = 5000,
  ) {}

  subscribe(fn: Listener): () => void {
    this.listeners.add(fn);
    if (this.listeners.size === 1) this.start();
    return () => {
      this.listeners.delete(fn);
      if (this.listeners.size === 0) this.stop();
    };
  }

  private start(): void {
    if (this.running) return;
    this.running = true;
    void this.tick();
  }

  stop(): void {
    this.running = false;
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
  }

  /** Poll now (after a key press, so the key catches up at once). */
  async refresh(): Promise<void> {
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
    await this.tick();
  }

  private async tick(): Promise<void> {
    if (!this.running) return;
    let next = this.fastMs;
    if (!this.client.paired) {
      this.emit(null, new PlatterError("unpaired", "Pair with Platter first."));
      next = this.slowMs;
    } else {
      try {
        this.last = await this.client.state();
        this.emit(this.last, null);
      } catch (e) {
        const err = e instanceof PlatterError ? e : new PlatterError("failed", String(e));
        this.last = null;
        this.emit(null, err);
        if (err.kind !== "busy") next = this.slowMs;
      }
    }
    if (this.running && !this.timer) this.timer = setTimeout(() => {
      this.timer = null;
      void this.tick();
    }, next);
  }

  private emit(s: PlatterState | null, err: PlatterError | null): void {
    for (const fn of this.listeners) fn(s, err);
  }
}
