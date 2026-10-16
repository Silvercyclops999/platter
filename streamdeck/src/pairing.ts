/**
 * Pairing without a code. On first connect with no token the plugin asks
 * Platter, which brings its window up with "Allow Stream Deck to control
 * Platter?". The keys say "Approve in Platter" until the person answers;
 * Allow gives a token that every key then uses. After Don't allow (or no
 * answer) it only asks again when a key is pressed. No Stream Deck code in
 * here, so it can be tested on its own.
 */
import { type PlatterClient, PlatterError } from "./client.js";

/** What the pairer is doing. */
export type Phase = "idle" | "asking" | "waiting" | "denied";

export interface PairerHooks {
  /** Allowed: keep the token (the client already uses it). */
  onToken(token: string): void | Promise<void>;
  /** The open request's id, kept so a restart can pick it up (null when done). */
  onRequest?(id: string | null): void | Promise<void>;
  /** The phase changed, so the keys can redraw. */
  onChange?(phase: Phase): void;
}

export class AutoPair {
  phase: Phase = "idle";
  /** Asks by itself until Platter has shown the prompt once. */
  private eager = false;
  private request: string | null = null;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private since = 0;
  private lastAsk = -Infinity;
  /** Why the last ask didn't reach Platter. */
  private unreachable: PlatterError | null = null;

  constructor(
    private readonly client: PlatterClient,
    private readonly hooks: PairerHooks,
    /** How often to check for an answer. */
    readonly checkMs = 1000,
    /** How often to ask again while Platter is busy, or not running yet. */
    readonly retryMs = 5000,
    /** How long to wait for an answer (Platter's prompt goes away after two minutes). */
    readonly waitMs = 130_000,
  ) {}

  /** Connected. With no token, ask Platter (or pick up a request from before a restart). */
  start(request: string | null = null): void {
    if (this.client.paired) return;
    if (request) {
      this.request = request;
      this.since = Date.now();
      this.set("waiting");
      this.schedule(0);
      return;
    }
    this.eager = true;
    this.auto();
  }

  /** From the state poller while unpaired: ask, if it still asks by itself (not more than every retryMs). */
  auto(): void {
    if (this.eager && this.phase === "idle" && Date.now() - this.lastAsk >= this.retryMs) void this.ask();
  }

  /** A key was pressed while unpaired (or the settings panel asked): ask again. */
  press(): void {
    if (this.phase === "idle" || this.phase === "denied") void this.ask();
  }

  /** What the keys should say while unpaired. */
  problem(): PlatterError | null {
    if (this.phase === "asking" || this.phase === "waiting") return new PlatterError("approve", "Approve in Platter.");
    if (this.phase === "denied") return new PlatterError("denied", "Not allowed in Platter. Press a key to ask again.");
    return this.unreachable;
  }

  stop(): void {
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
  }

  private set(phase: Phase): void {
    if (this.phase === phase) return;
    this.phase = phase;
    this.hooks.onChange?.(phase);
  }

  private schedule(ms: number): void {
    this.stop();
    this.timer = setTimeout(() => {
      this.timer = null;
      void this.check();
    }, ms);
  }

  private async ask(): Promise<void> {
    this.stop();
    this.lastAsk = Date.now();
    this.set("asking");
    try {
      this.request = await this.client.requestPairing();
    } catch (e) {
      const kind = e instanceof PlatterError ? e.kind : "failed";
      // Platter is already asking, or this asked too soon: try again in a
      // while, and the keys keep saying "Approve in Platter".
      if (kind === "approve" || kind === "busy") {
        this.timer = setTimeout(() => {
          this.timer = null;
          if (this.phase === "asking") void this.ask();
        }, this.retryMs);
        return;
      }
      // Not running yet, or Allow Stream Deck is off: the state poller asks
      // again on its next try while this still asks by itself.
      this.unreachable = e instanceof PlatterError && (kind === "offline" || kind === "refused") ? e : null;
      this.set("idle");
      return;
    }
    this.unreachable = null;
    this.eager = false;
    this.since = Date.now();
    await this.hooks.onRequest?.(this.request);
    this.set("waiting");
    this.schedule(this.checkMs);
  }

  private async check(): Promise<void> {
    const id = this.request;
    if (!id) return;
    let answer: Awaited<ReturnType<PlatterClient["pairStatus"]>> | null = null;
    try {
      answer = await this.client.pairStatus(id);
    } catch {
      answer = null;
    }
    if (answer?.state === "waiting" || (answer === null && Date.now() - this.since < this.waitMs)) {
      this.schedule(this.checkMs);
      return;
    }
    this.request = null;
    this.eager = false;
    await this.hooks.onRequest?.(null);
    if (answer?.state === "allowed") {
      await this.hooks.onToken(answer.token);
      this.set("idle");
    } else if (answer?.state === "denied") {
      this.set("denied");
    } else {
      this.set("idle");
    }
  }
}
