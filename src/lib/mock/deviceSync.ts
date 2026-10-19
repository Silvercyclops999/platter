/**
 * Mock Keep my devices in sync: paired devices, pairing codes, held
 * changes and download suggestions. Device names are invented.
 *
 * Test hooks: `otherCode` (the 6 digits the pretend other device shows),
 * `otherLong` (its long code), `away` (no device on this network),
 * `otherAccount` (the other device is signed in to another account),
 * `relayDown` (the relay can't be reached), `copyFails` (the computer
 * doesn't answer a settings copy), `statusFails` (the status can't be
 * read), `pairArrives(name)` (a device joins the codes this one shows),
 * `pairAsks(name)` (a device used the long code and waits for a yes),
 * `expirePair()`, `hold(n)` (changes held from
 * a clock far ahead), `suggest(...)`.
 */
import type {
  DevicePairState,
  DevicePaired,
  DeviceSyncDevice,
  DeviceSyncOverview,
  DownloadKind,
  DownloadSuggestion,
  Settings,
  SyncQuarantined,
} from "../types";
import { mockQr } from "./sync";

export interface DeviceSyncHooks {
  settings: () => Settings;
  setSettings: (patch: Partial<Settings>) => void;
  emitStatus: (s: DeviceSyncDevice[]) => void;
  emitPair: (s: DevicePairState) => void;
}

const TTL_MS = 10 * 60 * 1000;
const IDLE: DevicePairState = { phase: "idle", code: null, longCode: null, qr: null, expiresAt: null, with: null };
const LONG = "PB1-q3Vx8bN2kLm9TzR4wYc7HdE1fGj5sAo6pUi0eWn3vXb8yZaQk";

export class MockDeviceSync {
  otherCode = "135790";
  otherLong = LONG;
  away = false;
  otherAccount = false;
  relayDown = false;
  copyFails = false;
  statusFails = false;
  devices: DeviceSyncDevice[] = [];
  held: SyncQuarantined[] = [];
  suggestions: DownloadSuggestion[] = [];
  private pair: DevicePairState = { ...IDLE };
  private seq = 0;

  constructor(private hooks: DeviceSyncHooks) {}

  reset() {
    this.otherCode = "135790";
    this.otherLong = LONG;
    this.away = false;
    this.otherAccount = false;
    this.relayDown = false;
    this.copyFails = false;
    this.statusFails = false;
    this.devices = [];
    this.held = [];
    this.suggestions = [];
    this.pair = { ...IDLE };
  }

  private on() {
    return this.hooks.settings().deviceSync !== false;
  }

  private words(d: DeviceSyncDevice): DeviceSyncDevice {
    return this.on() ? d : { ...d, words: "Keep my devices in sync is off" };
  }

  overview(): DeviceSyncOverview {
    if (this.statusFails) throw { kind: "internal", message: "database is locked" };
    return { on: this.on(), devices: this.devices.map((d) => this.words(d)), quarantined: this.held.length, pairing: this.pairState() };
  }

  private changed() {
    this.hooks.emitStatus(this.devices.map((d) => this.words(d)));
  }

  private add(name: string): DevicePaired {
    const id = `dev-${++this.seq}`;
    this.devices = [...this.devices, { id, name, lastSyncedAt: Date.now(), words: "In sync", updateOther: null, clock: null, held: 0 }];
    this.changed();
    return { id, name };
  }

  /** Test hook: a device of this account in step, or in another state. */
  addDevice(name: string, d: Partial<DeviceSyncDevice> = {}) {
    const id = `dev-${++this.seq}`;
    this.devices = [...this.devices, { id, name, lastSyncedAt: Date.now() - 60_000, words: "In sync", updateOther: null, clock: null, held: 0, ...d }];
    this.changed();
    return id;
  }

  /** Test hook: changes held from a device whose clock is far ahead. */
  hold(n: number, from = "Studio Mac") {
    for (let i = 0; i < n; i++) {
      this.held.push({ ns: "hidden", key: `track:${i}`, entry: { value: { kind: "track", title: `Held ${i}` }, ts: Date.now() + 3 * 86_400_000, device: "mac" }, from });
    }
  }

  /** Test hook: another device downloaded this. */
  suggest(kind: DownloadKind, id: string, title: string, devices = ["Studio PC"]) {
    this.suggestions.push({ kind, id, title, devices, addedAt: Date.now() });
  }

  syncNow(): DeviceSyncOverview {
    if (!this.on()) throw { kind: "invalid", message: "Turn on Keep my devices in sync first." };
    this.devices = this.devices.map((d) =>
      this.relayDown && d.words !== "In sync" ? { ...d, words: `${d.name} is away, changes are kept and will go when it's back` } : { ...d, lastSyncedAt: Date.now(), words: d.updateOther ? d.words : "In sync" },
    );
    this.changed();
    return this.overview();
  }

  forget(id: string): DeviceSyncOverview {
    this.devices = this.devices.filter((d) => d.id !== id);
    this.changed();
    return this.overview();
  }

  takeHeld(): DeviceSyncOverview {
    this.held = [];
    return this.overview();
  }

  pairState(): DevicePairState {
    if (this.pair.phase === "waiting" && this.pair.expiresAt && Date.now() >= this.pair.expiresAt) this.pair = { ...IDLE, phase: "expired" };
    return this.pair;
  }

  pairStart(): DevicePairState {
    const code = String(100000 + Math.floor(Math.random() * 900000));
    this.pair = this.relayDown
      ? {
          phase: "waiting",
          code,
          longCode: null,
          qr: null,
          expiresAt: Date.now() + TTL_MS,
          with: null,
          note: "The relay can't be reached. The 6-digit code still works on this network.",
        }
      : { phase: "waiting", code, longCode: LONG, qr: mockQr(LONG), expiresAt: Date.now() + TTL_MS, with: null };
    this.hooks.emitPair(this.pair);
    return this.pair;
  }

  pairCancel() {
    this.pair = { ...IDLE };
    this.hooks.emitPair(this.pair);
  }

  /** Test/demo hook: another device used the codes this one shows. */
  pairArrives(name = "Studio PC") {
    if (this.pairState().phase !== "waiting") return;
    this.add(name);
    this.pair = { ...IDLE, phase: "paired", with: name };
    this.hooks.emitPair(this.pair);
  }

  /** Test/demo hook: a device used the long code and waits for a yes here. */
  pairAsks(name = "Studio PC") {
    if (this.pairState().phase !== "waiting") return;
    this.pair = { ...this.pair, phase: "confirm", code: null, longCode: null, qr: null, with: name };
    this.hooks.emitPair(this.pair);
  }

  pairConfirm(yes: boolean): DevicePairState {
    if (this.pair.phase !== "confirm") throw { kind: "notFound", message: "No device is asking to pair." };
    const name = this.pair.with ?? "Studio PC";
    if (yes) this.add(name);
    this.pair = yes ? { ...IDLE, phase: "paired", with: name } : { ...IDLE, phase: "declined" };
    this.hooks.emitPair(this.pair);
    return this.pair;
  }

  /** Test hook: the codes ran out. */
  expirePair() {
    if (this.pair.phase === "waiting") {
      this.pair = { ...IDLE, phase: "expired" };
      this.hooks.emitPair(this.pair);
    }
  }

  join(raw: string): DevicePaired {
    const code = raw.replace(/[\s-]/g, "");
    if (/^\d{6}$/.test(code)) {
      if (this.away)
        throw { kind: "notFound", message: "No device on this network is showing a code. Away from home, use the long code or scan its QR code instead." };
      if (this.otherAccount) throw { kind: "auth", message: "That device is signed in to a different Plex account or Home user." };
      if (code !== this.otherCode) throw { kind: "invalid", message: "That code doesn't match. Check the code on the other device." };
      return this.add("Studio PC");
    }
    const long = raw.trim().replace(/^platter:\/\/pair-device\//, "");
    if (!long.startsWith("PB1-")) throw { kind: "invalid", message: "That isn't a pairing code. Type the 6 digits, or paste the long code." };
    if (this.relayDown) throw { kind: "network", message: "The relay can't be reached. Check the connection and try again." };
    if (this.otherAccount || long !== this.otherLong)
      throw { kind: "network", message: "Your other device didn't answer. Keep its code showing, check it's online, and try again." };
    return this.add("Studio PC");
  }

  dismiss(kind: DownloadKind, id: string | null) {
    this.suggestions = this.suggestions.filter((s) => !(s.kind === kind && s.id === (id ?? "")));
  }

  copySettings(): string {
    if (!this.on()) throw { kind: "invalid", message: "Turn on Keep my devices in sync first." };
    const pc = this.devices[0];
    if (!pc) throw { kind: "notFound", message: "Pair this device with your computer first, in Settings under Keep my devices in sync." };
    if (this.copyFails) throw { kind: "network", message: `${pc.name} didn't answer. Open Platter there and try again.` };
    this.hooks.setSettings({ accent: "brass", theme: "studio" });
    return pc.name;
  }
}
