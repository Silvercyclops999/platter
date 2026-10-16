/**
 * The Stream Deck plugin: keys, dials and the Neo info bar for Platter,
 * over its local control API. The pairing token is kept in the plugin's
 * global settings. Pictures come from views.ts and dials.ts; what a press
 * sends comes from behaviour.ts.
 */
import streamDeck, {
  type DialAction,
  type DialDownEvent,
  type DialRotateEvent,
  type DialUpEvent,
  type DidReceiveSettingsEvent,
  type KeyAction,
  type KeyDownEvent,
  type KeyUpEvent,
  type SendToPluginEvent,
  SingletonAction,
  type TouchTapEvent,
  type WillAppearEvent,
  type WillDisappearEvent,
} from "@elgato/streamdeck";
import type { JsonObject } from "@elgato/utils";
import { pressCommands, REPEATS, GESTURES, SHOWS_OK } from "./behaviour.js";
import { type Command, type Device, httpTransport, type LibraryItem, type LibraryKind, PlatterClient, PlatterError, type PlatterState, type QueueItem, StatePoller } from "./client.js";
import { type DialId, type DialLists, type DialModel, dialImage, dialPress, dialTurn, newDial } from "./dials.js";
import { sliceFor } from "./groups.js";
import { Live, type Snapshot } from "./live.js";
import { InfoOutput } from "./output.js";
import { AutoPair } from "./pairing.js";
import { neoBar } from "./render.js";
import { DEFAULTS, formatInfo, settingsFor } from "./settings.js";
import { DIAL_DOUBLE_MS, HoldRepeat, Overlay, Presses } from "./timing.js";
import { infoFields, KEY_ACTIONS, type KeyActionId, keyView } from "./views.js";

type Globals = { token?: string; request?: string };

const PREFIX = "app.platter.streamdeck.";
const client = new PlatterClient(httpTransport());
// Pairing without a code: ask Platter, and keep the token it gives for every key.
const pairer: AutoPair = new AutoPair(client, {
  onToken: (token) => streamDeck.settings.setGlobalSettings<Globals>({ token }),
  onRequest: (request) => streamDeck.settings.setGlobalSettings<Globals>(request ? { request } : {}),
  onChange: () => void poller.refresh(),
});
const poller = new StatePoller(client, undefined, undefined, pairer);
const live = new Live();
let snap: Snapshot = live.snapshot;

/** Everything drawn on a device redraws through these. */
const redrawers = new Set<() => void>();
const redrawAll = () => {
  for (const r of redrawers) r();
};

// ---------------------------------------------------------------- data

/** Library lists (play keys, playlist targets, the dial browser), refreshed now and then. */
const libraries = new Map<LibraryKind, { items: LibraryItem[]; at: number }>();
const loading = new Set<string>();
const LIBRARY_TTL_MS = 5 * 60 * 1000;

function library(kind: LibraryKind): LibraryItem[] {
  const have = libraries.get(kind);
  if ((!have || Date.now() - have.at > LIBRARY_TTL_MS) && !loading.has(kind) && snap.conn === "ok") {
    loading.add(kind);
    void client
      .library(kind)
      .then((items) => libraries.set(kind, { items, at: Date.now() }))
      .catch(() => libraries.set(kind, { items: have?.items ?? [], at: Date.now() - LIBRARY_TTL_MS + 30_000 }))
      .finally(() => {
        loading.delete(kind);
        redrawAll();
      });
  }
  return have?.items ?? [];
}

/** A cover by key: kept, or fetched and drawn when it arrives. */
function cover(key: string | null | undefined): string | null {
  if (!key) return null;
  const kept = client.coverKept(key);
  if (kept) return kept;
  if (!loading.has(`cover:${key}`) && snap.conn === "ok") {
    loading.add(`cover:${key}`);
    void client.coverFor(key).finally(() => {
      loading.delete(`cover:${key}`);
      redrawAll();
    });
  }
  return null;
}

const item = (kind: LibraryKind, id: string) => library(kind).find((i) => i.id === id);

/** Send commands; flash an alert when one fails, OK when asked. Not paired: ask Platter instead. */
async function run(cmds: Command[], key?: { showAlert(): Promise<void>; showOk?(): Promise<void> }): Promise<void> {
  if (cmds.length === 0) {
    await key?.showAlert();
    return;
  }
  for (const cmd of cmds) {
    try {
      await client.command(cmd);
      if (SHOWS_OK.has(cmd.command)) await key?.showOk?.();
    } catch (e) {
      streamDeck.logger.info(`${cmd.command} failed: ${e instanceof Error ? e.message : String(e)}`);
      // Not paired (or no longer): pressing a key asks Platter again.
      if (e instanceof PlatterError && e.kind === "unpaired") {
        pairer.press();
        break;
      }
      await key?.showAlert();
    }
  }
  await poller.refresh();
}

// ---------------------------------------------------------------- live state

let watching: (() => void) | null = null;
let ticker: ReturnType<typeof setInterval> | null = null;
const info = new InfoOutput();
let lastSong: string | null = null;

/** Start polling once anything shows; the elapsed time ticks each second. */
function watch(): void {
  watching ??= poller.subscribe((s: PlatterState | null, err: PlatterError | null) => {
    snap = live.update(s, err);
    const song = snap.state?.trackId ?? null;
    if (song !== lastSong) {
      lastSong = song;
      dialListsStale = true;
      writeOutputs();
    }
    redrawAll();
  });
  ticker ??= setInterval(redrawAll, 1000);
}

function unwatchIfIdle(): void {
  if (redrawers.size > 0) return;
  watching?.();
  watching = null;
  if (ticker) clearInterval(ticker);
  ticker = null;
}

// ---------------------------------------------------------------- keys

interface KeyCtx {
  action: KeyAction<JsonObject>;
  settings: Record<string, unknown>;
  presses: Presses | null;
  hold: HoldRepeat | null;
  sig: string;
}

/** Song info keys writing to the clipboard or a file. */
const outputs = new Map<string, Record<string, unknown>>();

function writeOutputs(): void {
  for (const st of outputs.values()) {
    const o = st as unknown as ReturnType<typeof settingsFor<"songinfo">>;
    if (o.output === "none") continue;
    const text = formatInfo(o.format, infoFields(snap, Date.now()));
    void info.put(o.output, o.file, text).catch((e) => streamDeck.logger.info(`song info not written: ${String(e)}`));
  }
}

/** Play / Pause keys per device, for big art. */
const playKeys = new Map<string, { device: string; column: number; row: number }>();

class KeyAct extends SingletonAction<JsonObject> {
  override readonly manifestId: string;
  private ctxs = new Map<string, KeyCtx>();

  constructor(readonly id: KeyActionId) {
    super();
    this.manifestId = PREFIX + id;
  }

  private settings(raw: unknown): Record<string, unknown> {
    return this.id in DEFAULTS ? (settingsFor(this.id as keyof typeof DEFAULTS, raw) as unknown as Record<string, unknown>) : { images: settingsFor("previous", raw).images };
  }

  private draw = (): void => {
    const now = Date.now();
    for (const [ctxId, c] of this.ctxs) {
      let slice = null;
      if (this.id === "playpause") {
        const me = playKeys.get(ctxId);
        const st = c.settings as unknown as ReturnType<typeof settingsFor<"playpause">>;
        slice = sliceFor(st, me ?? null, [...playKeys.values()].filter((k) => k.device === me?.device));
      }
      const v = keyView(this.id, c.settings, { snap, now, cover, item, slice });
      const sig = `${v.title}\n${v.image}`;
      if (sig === c.sig) continue;
      c.sig = sig;
      void c.action.setImage(v.image);
      void c.action.setTitle(v.title);
    }
  };

  override onWillAppear(ev: WillAppearEvent<JsonObject>): void {
    if (!ev.action.isKey()) return;
    const k = ev.action;
    const g = GESTURES[this.id];
    const c: KeyCtx = { action: k, settings: this.settings(ev.payload.settings), presses: null, hold: null, sig: "" };
    if (g) c.presses = new Presses((gesture) => void run(pressCommands(this.id, gesture, c.settings, snap), k), { double: !!g.double, long: !!g.long });
    if (REPEATS.includes(this.id)) c.hold = new HoldRepeat(() => void run(pressCommands(this.id, "single", c.settings, snap), k));
    this.ctxs.set(k.id, c);
    if (this.id === "playpause" && k.coordinates) playKeys.set(k.id, { device: k.device.id, ...k.coordinates });
    if (this.id === "songinfo") {
      outputs.set(k.id, c.settings);
      info.forget();
    }
    redrawers.add(this.draw);
    watch();
    void poller.refresh();
    if (this.id === "playpause") redrawAll();
  }

  override onWillDisappear(ev: WillDisappearEvent<JsonObject>): void {
    const c = this.ctxs.get(ev.action.id);
    c?.presses?.reset();
    c?.hold?.up();
    this.ctxs.delete(ev.action.id);
    playKeys.delete(ev.action.id);
    outputs.delete(ev.action.id);
    if (this.ctxs.size === 0) redrawers.delete(this.draw);
    if (this.id === "playpause") redrawAll();
    unwatchIfIdle();
  }

  override onDidReceiveSettings(ev: DidReceiveSettingsEvent<JsonObject>): void {
    const c = this.ctxs.get(ev.action.id);
    if (!c) return;
    c.settings = this.settings(ev.payload.settings);
    c.sig = "";
    if (this.id === "songinfo") {
      outputs.set(ev.action.id, c.settings);
      info.forget();
      writeOutputs();
    }
    if (this.id === "playpause") redrawAll();
    else this.draw();
  }

  override onKeyDown(ev: KeyDownEvent<JsonObject>): Promise<void> | void {
    const c = this.ctxs.get(ev.action.id);
    if (!c) return;
    const multi = ev.payload.isInMultiAction;
    if (c.hold) return c.hold.down(multi);
    if (c.presses) return c.presses.down(multi);
    return run(pressCommands(this.id, "single", c.settings, snap), ev.action);
  }

  override onKeyUp(ev: KeyUpEvent<JsonObject>): void {
    const c = this.ctxs.get(ev.action.id);
    c?.hold?.up();
    c?.presses?.up();
  }

  override onSendToPlugin(ev: SendToPluginEvent<JsonObject, JsonObject>): Promise<void> | void {
    return answerList(ev.payload);
  }
}

/** A settings panel asks for a list to pick from: `{ list: "playlists" | ... | "devices" }`. */
async function answerList(payload: unknown): Promise<void> {
  const p = payload as { list?: string } | null;
  const list = p?.list;
  if (!list) return;
  try {
    if (list === "devices") {
      const items = await client.devices();
      await streamDeck.ui.sendToPropertyInspector({ list, items: items.map((d) => ({ id: d.id, title: d.name })) });
      return;
    }
    if (!["playlists", "albums", "artists", "mixes"].includes(list)) return;
    const items = await client.library(list as LibraryKind);
    libraries.set(list as LibraryKind, { items, at: Date.now() });
    await streamDeck.ui.sendToPropertyInspector({ list, items: items.map((i) => ({ id: i.id, title: i.title, subtitle: i.subtitle })) });
  } catch (e) {
    await streamDeck.ui.sendToPropertyInspector({ list, items: [], error: e instanceof Error ? e.message : String(e) });
  }
}

// ---------------------------------------------------------------- dials

interface DialCtx {
  action: DialAction<JsonObject>;
  settings: Record<string, unknown>;
  model: DialModel;
  overlay: Overlay;
  /** When the last press went down, for double presses. */
  lastPress: number;
  pressTimer: ReturnType<typeof setTimeout> | null;
  sig: string;
}

const dialLists: DialLists = { browse: [], queue: [], devices: [] };
let dialListsStale = true;
let dialListsAt = 0;

/** Queue and devices for the dials, fetched when the song changes or every 15 s. */
function refreshDialLists(want: { queue: boolean; devices: boolean }): void {
  if (snap.conn !== "ok" || loading.has("dials")) return;
  if (!dialListsStale && Date.now() - dialListsAt < 15_000) return;
  loading.add("dials");
  dialListsStale = false;
  dialListsAt = Date.now();
  void Promise.all([
    want.queue ? client.queue().catch((): QueueItem[] => dialLists.queue) : Promise.resolve(dialLists.queue),
    want.devices ? client.devices().catch((): Device[] => dialLists.devices) : Promise.resolve(dialLists.devices),
  ])
    .then(([q, d]) => {
      dialLists.queue = q;
      dialLists.devices = d;
    })
    .finally(() => {
      loading.delete("dials");
      redrawAll();
    });
}

class DialAct extends SingletonAction<JsonObject> {
  override readonly manifestId: string;
  private ctxs = new Map<string, DialCtx>();

  constructor(readonly id: DialId) {
    super();
    this.manifestId = PREFIX + id;
  }

  private settings(raw: unknown): Record<string, unknown> {
    if (this.id === "volume") return settingsFor("volume", raw) as unknown as Record<string, unknown>;
    if (this.id === "volumedial") return settingsFor("volumedial", raw) as unknown as Record<string, unknown>;
    return {};
  }

  private lists(c: DialCtx): DialLists {
    const browse = this.id === "volume" ? library((c.settings as { browse: LibraryKind }).browse) : [];
    return { ...dialLists, browse };
  }

  private draw = (): void => {
    const now = Date.now();
    if (this.id === "queuedial" || this.id === "listenondial") refreshDialLists({ queue: this.id === "queuedial", devices: this.id === "listenondial" });
    for (const c of this.ctxs.values()) {
      const lists = this.lists(c);
      const len = this.id === "queuedial" ? lists.queue.length : this.id === "listenondial" ? lists.devices.length : c.model.view === "browse" ? lists.browse.length : 0;
      if (c.model.index >= len) c.model.index = Math.max(0, len - 1);
      const img = dialImage(this.id, c.settings, c.model, snap, now, lists, cover);
      const sig = img;
      if (sig === c.sig) continue;
      c.sig = sig;
      void c.action.setFeedback({ canvas: img });
    }
  };

  private redraw(c: DialCtx): void {
    c.sig = "";
    this.draw();
  }

  override onWillAppear(ev: WillAppearEvent<JsonObject>): void {
    if (!ev.action.isDial()) return;
    const a = ev.action;
    const c: DialCtx = {
      action: a,
      settings: this.settings(ev.payload.settings),
      model: newDial(),
      overlay: new Overlay((visible) => {
        c.model.overlay = visible;
        this.redraw(c);
      }),
      lastPress: 0,
      pressTimer: null,
      sig: "",
    };
    this.ctxs.set(a.id, c);
    void a.setFeedbackLayout("layouts/canvas.json");
    dialListsStale = true;
    redrawers.add(this.draw);
    watch();
    void poller.refresh();
  }

  override onWillDisappear(ev: WillDisappearEvent<JsonObject>): void {
    const c = this.ctxs.get(ev.action.id);
    c?.overlay.hide();
    if (c?.pressTimer) clearTimeout(c.pressTimer);
    this.ctxs.delete(ev.action.id);
    if (this.ctxs.size === 0) redrawers.delete(this.draw);
    unwatchIfIdle();
  }

  override onDidReceiveSettings(ev: DidReceiveSettingsEvent<JsonObject>): void {
    const c = this.ctxs.get(ev.action.id);
    if (!c) return;
    c.settings = this.settings(ev.payload.settings);
    this.redraw(c);
  }

  override async onDialRotate(ev: DialRotateEvent<JsonObject>): Promise<void> {
    const c = this.ctxs.get(ev.action.id);
    if (!c) return;
    const cmd = dialTurn(this.id, c.settings, c.model, ev.payload.ticks, this.lists(c));
    if (c.model.overlay) c.overlay.poke();
    this.redraw(c);
    if (cmd) await run([cmd], ev.action);
  }

  /** Press or touch: Multimedia waits a moment for a second press (switch view). */
  private press(c: DialCtx, key: { showAlert(): Promise<void> }): void {
    if (this.id !== "volume") {
      const cmd = dialPress(this.id, c.settings, c.model, false, this.lists(c));
      if (cmd) void run([cmd], key);
      return;
    }
    if (c.pressTimer) {
      clearTimeout(c.pressTimer);
      c.pressTimer = null;
      dialPress(this.id, c.settings, c.model, true, this.lists(c));
      this.redraw(c);
      return;
    }
    c.pressTimer = setTimeout(() => {
      c.pressTimer = null;
      const cmd = dialPress(this.id, c.settings, c.model, false, this.lists(c));
      if (cmd) void run([cmd], key);
    }, DIAL_DOUBLE_MS);
  }

  override onDialDown(ev: DialDownEvent<JsonObject>): void {
    const c = this.ctxs.get(ev.action.id);
    if (c) this.press(c, ev.action);
  }

  override onDialUp(_ev: DialUpEvent<JsonObject>): void {}

  override onTouchTap(ev: TouchTapEvent<JsonObject>): void {
    const c = this.ctxs.get(ev.action.id);
    if (!c) return;
    // Touch: play / pause in the player view, play the picked one in the browser.
    const cmd = dialPress(this.id, c.settings, c.model, false, this.lists(c));
    if (cmd) void run([cmd], ev.action);
  }

  override onSendToPlugin(ev: SendToPluginEvent<JsonObject, JsonObject>): Promise<void> | void {
    return answerList(ev.payload);
  }
}

// ---------------------------------------------------------------- Neo

class NowPlayingBar extends SingletonAction<JsonObject> {
  override readonly manifestId = `${PREFIX}nowplaying`;
  private sig = "";

  private draw = (): void => {
    const now = Date.now();
    const s = snap.state;
    const song = !!s && (!!s.trackId || s.durationMs > 0);
    const img =
      snap.conn === "offline"
        ? neoBar({ cover: null, title: "Platter not running", artist: "-", progress: 0, paused: true })
        : snap.conn === "approve"
          ? neoBar({ cover: null, title: "Approve in Platter", artist: "Choose Allow in Platter's window", progress: 0, paused: true })
          : neoBar({
            cover: song ? (s?.cover ?? null) : null,
            title: song ? (s?.private ? "Private session" : (s?.title ?? "")) : "Nothing playing",
            artist: song ? (s?.artist ?? "-") : "-",
            progress: song && s && s.durationMs > 0 ? Math.min(1, (s.positionMs + (s.playing && !snap.held ? now - snap.at : 0)) / s.durationMs) : 0,
            paused: !s?.playing,
          });
    const sig = img;
    if (sig === this.sig) return;
    this.sig = sig;
    for (const a of this.actions) if (a.isNeoInfobar()) void a.setFeedback({ canvas: img });
  };

  override onWillAppear(ev: WillAppearEvent<JsonObject>): void {
    if (ev.action.isNeoInfobar()) void ev.action.setFeedbackLayout("layouts/neo.json");
    this.sig = "";
    redrawers.add(this.draw);
    watch();
    void poller.refresh();
  }

  override onWillDisappear(): void {
    if ([...this.actions].length <= 1) redrawers.delete(this.draw);
    unwatchIfIdle();
  }
}

// Pairing, from the property inspector: { ask: true } asks Platter,
// { pair: "123456" } uses the code, { status: true } reports.
streamDeck.ui.onSendToPlugin<{ pair?: string; status?: boolean; unpair?: boolean; ask?: boolean }>(async (ev) => {
  const p = ev.payload;
  const reply = (payload: { paired: boolean; reachable: boolean; asking?: boolean; error?: string }) =>
    streamDeck.ui.sendToPropertyInspector(payload);
  if (p?.unpair) {
    client.setToken(null);
    await streamDeck.settings.setGlobalSettings<Globals>({});
  }
  if (p?.ask) {
    pairer.press();
    await reply({ paired: false, reachable: true, asking: true });
    return;
  }
  if (typeof p?.pair === "string") {
    try {
      const token = await client.pair(p.pair);
      await streamDeck.settings.setGlobalSettings<Globals>({ token });
      await reply({ paired: true, reachable: true });
      await poller.refresh();
    } catch (e) {
      await reply({ paired: false, reachable: !(e instanceof PlatterError && e.kind === "offline"), error: e instanceof Error ? e.message : String(e) });
    }
    return;
  }
  try {
    const h = await client.hello();
    await reply({ paired: h.paired, reachable: true, asking: !h.paired && (pairer.phase === "asking" || pairer.phase === "waiting") });
  } catch (e) {
    await reply({ paired: false, reachable: false, error: e instanceof Error ? e.message : String(e) });
  }
});

for (const id of KEY_ACTIONS) streamDeck.actions.registerAction(new KeyAct(id));
for (const id of ["volume", "playpausedial", "volumedial", "queuedial", "listenondial"] as const) streamDeck.actions.registerAction(new DialAct(id));
streamDeck.actions.registerAction(new NowPlayingBar());

void streamDeck.connect().then(async () => {
  const g = await streamDeck.settings.getGlobalSettings<Globals>();
  client.setToken(g.token ?? null);
  // First connect with no token: ask Platter to pair.
  pairer.start(g.token ? null : (g.request ?? null));
  await poller.refresh();
});
