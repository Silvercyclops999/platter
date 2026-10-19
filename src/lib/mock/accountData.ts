/**
 * The browser mock's connected streaming account: signing in always works
 * unless a test says otherwise, and a read imports two made-up playlists
 * through the import mock.
 */
import type { AccountState, AccountStatus } from "../importTypes";

export const REDIRECT_URI = "http://127.0.0.1/callback";

/** What a test can make the next sign-in or read do. */
export type AccountOutcome =
  | "ok"
  | "deny"
  | "invalidClient"
  | "refused"
  | "expired"
  | "rateLimited"
  | "offline";

export class MockAccount {
  signedIn = false;
  /** Kept in the keychain by the core; never sent to the UI. */
  clientId = "";
  /** Test hooks. */
  nextSignIn: AccountOutcome = "ok";
  nextRead: AccountOutcome = "ok";
  /** Sign-ins and reads stay pending until `finish()` when set. */
  hold = false;
  private phase: {
    state: AccountState;
    message: string | null;
    retryAt: number | null;
  } | null = null;
  private name: string | null = null;
  private pending: (() => void) | null = null;

  constructor(
    private enabled: () => boolean,
    private importRead: () => number,
    private lastRead: () => number | null,
  ) {}

  reset() {
    this.signedIn = false;
    this.clientId = "";
    this.nextSignIn = "ok";
    this.nextRead = "ok";
    this.hold = false;
    this.phase = null;
    this.name = null;
    this.pending = null;
  }

  status(): AccountStatus {
    const base: AccountStatus = {
      state: "signedOut",
      redirectUri: REDIRECT_URI,
      name: this.signedIn ? this.name : null,
      message: null,
      retryAt: null,
      done: 0,
      total: 0,
      lastRead: this.lastRead(),
    };
    if (!this.clientId) return { ...base, state: "notSetUp", name: null };
    if (this.phase) {
      if (
        this.phase.state === "rateLimited" &&
        (this.phase.retryAt ?? 0) <= Date.now()
      )
        this.phase = null;
      else
        return {
          ...base,
          ...this.phase,
          ...(this.phase.state === "reading" ? { done: 1, total: 3 } : {}),
        };
    }
    return { ...base, state: this.signedIn ? "connected" : "signedOut" };
  }

  /** Lets a held sign-in or read go on. */
  finish() {
    const p = this.pending;
    this.pending = null;
    p?.();
  }

  private wait(): Promise<void> {
    if (!this.hold) return Promise.resolve();
    return new Promise((r) => (this.pending = r));
  }

  async signIn(): Promise<AccountStatus> {
    if (!this.clientId || !this.enabled()) return this.status();
    this.phase = { state: "signingIn", message: null, retryAt: null };
    await this.wait();
    if (this.phase?.state !== "signingIn") return this.status();
    const o = this.nextSignIn;
    this.nextSignIn = "ok";
    if (o === "deny")
      this.phase = {
        state: "refused",
        message: "You didn't allow Platter to read your playlists.",
        retryAt: null,
      };
    else if (o === "invalidClient")
      this.phase = {
        state: "refused",
        message: "The Client ID wasn't accepted. Check it in Settings.",
        retryAt: null,
      };
    else {
      this.phase = null;
      this.signedIn = true;
      this.name = "Sam";
    }
    return this.status();
  }

  async read(): Promise<AccountStatus> {
    if (!this.clientId || !this.enabled() || !this.signedIn)
      return this.status();
    this.phase = { state: "reading", message: null, retryAt: null };
    await this.wait();
    if (this.phase?.state !== "reading") return this.status();
    const o = this.nextRead;
    this.nextRead = "ok";
    this.phase = null;
    if (o === "expired") {
      this.signedIn = false;
      this.phase = { state: "expired", message: null, retryAt: null };
    } else if (o === "rateLimited")
      this.phase = {
        state: "rateLimited",
        message: null,
        retryAt: Date.now() + 10 * 60_000,
      };
    else if (o === "offline")
      this.phase = {
        state: "offline",
        message:
          "You're offline. Platter reads your playlists again once you're back.",
        retryAt: null,
      };
    else if (o === "refused")
      this.phase = {
        state: "refused",
        message:
          "The service didn't let your account use this developer app. In the app's settings on the developer dashboard, add your account under User Management, then try again.",
        retryAt: null,
      };
    else this.importRead();
    return this.status();
  }

  cancel(): AccountStatus {
    if (this.phase?.state === "signingIn" || this.phase?.state === "reading")
      this.phase = null;
    this.finish();
    return this.status();
  }

  setClientId(id: string | null): AccountStatus {
    const next = (id ?? "").trim();
    if (next && !/^[A-Za-z0-9]{1,64}$/.test(next)) throw { kind: "invalid", message: "A Client ID is the letters and digits on your developer app's page." };
    if (next !== this.clientId) {
      this.signOut();
      this.clientId = next;
    }
    return this.status();
  }

  signOut(): AccountStatus {
    this.signedIn = false;
    this.phase = null;
    this.name = null;
    return this.status();
  }
}
