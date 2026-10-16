/**
 * The Stream Deck plugin: keys and a dial for Platter, over its local
 * control API. The pairing token is kept in the plugin's global settings.
 */
import streamDeck, {
  action,
  type DialAction,
  type DialDownEvent,
  type DialRotateEvent,
  type KeyAction,
  type KeyDownEvent,
  SingletonAction,
  type TouchTapEvent,
  type WillAppearEvent,
  type WillDisappearEvent,
} from "@elgato/streamdeck";
import { type Command, httpTransport, PlatterClient, PlatterError, type PlatterState, StatePoller } from "./client.js";
import { AutoPair } from "./pairing.js";
import { DIAL_STEP, type KeyView, likeView, plainView, playPauseView, volumeFeedback } from "./view.js";

type Globals = { token?: string; request?: string };

const client = new PlatterClient(httpTransport());
// Pairing without a code: ask Platter, and keep the token it gives for every key.
const pairer: AutoPair = new AutoPair(client, {
  onToken: (token) => streamDeck.settings.setGlobalSettings<Globals>({ token }),
  onRequest: (request) => streamDeck.settings.setGlobalSettings<Globals>(request ? { request } : {}),
  onChange: () => void poller.refresh(),
});
const poller = new StatePoller(client, undefined, undefined, pairer);

async function run(cmd: Command, key?: { showAlert(): Promise<void>; showOk?(): Promise<void> }, ok = false): Promise<void> {
  try {
    await client.command(cmd);
    if (ok) await key?.showOk?.();
  } catch (e) {
    streamDeck.logger.info(`${cmd.command} failed: ${e instanceof Error ? e.message : String(e)}`);
    // Not paired (or no longer): pressing a key asks Platter again.
    if (e instanceof PlatterError && e.kind === "unpaired") pairer.press();
    else await key?.showAlert();
  }
  await poller.refresh();
}

/** Keeps every visible key of one action in step with the state. */
abstract class Watching extends SingletonAction {
  private stop: (() => void) | null = null;
  private last: string | null = null;

  protected abstract view(s: PlatterState | null, err: PlatterError | null): KeyView;

  override onWillAppear(_ev: WillAppearEvent): void {
    this.last = null;
    this.stop ??= poller.subscribe((s, err) => void this.draw(s, err));
    void poller.refresh();
  }

  override onWillDisappear(_ev: WillDisappearEvent): void {
    if ([...this.actions].length <= 1) {
      this.stop?.();
      this.stop = null;
    }
  }

  private async draw(s: PlatterState | null, err: PlatterError | null): Promise<void> {
    const v = this.view(s, err);
    const sig = JSON.stringify([v.state, v.image?.length, v.image?.slice(-32), v.title]);
    if (sig === this.last) return;
    this.last = sig;
    for (const a of this.actions) {
      if (!a.isKey()) continue;
      const k = a as KeyAction;
      await k.setState(v.state);
      await k.setImage(v.image);
      await k.setTitle(v.title ?? "");
    }
  }
}

@action({ UUID: "app.platter.streamdeck.playpause" })
class PlayPause extends Watching {
  protected view = playPauseView;
  override onKeyDown(ev: KeyDownEvent): Promise<void> {
    return run({ command: "playPause" }, ev.action);
  }
}

@action({ UUID: "app.platter.streamdeck.next" })
class Next extends Watching {
  protected view = plainView;
  override onKeyDown(ev: KeyDownEvent): Promise<void> {
    return run({ command: "next" }, ev.action);
  }
}

@action({ UUID: "app.platter.streamdeck.previous" })
class Previous extends Watching {
  protected view = plainView;
  override onKeyDown(ev: KeyDownEvent): Promise<void> {
    return run({ command: "previous" }, ev.action);
  }
}

@action({ UUID: "app.platter.streamdeck.like" })
class Like extends Watching {
  protected view = likeView;
  override onKeyDown(ev: KeyDownEvent): Promise<void> {
    return run({ command: "like" }, ev.action);
  }
}

@action({ UUID: "app.platter.streamdeck.request" })
class RequestAlbum extends Watching {
  protected view = plainView;
  override onKeyDown(ev: KeyDownEvent): Promise<void> {
    return run({ command: "requestAlbum" }, ev.action, true);
  }
}

@action({ UUID: "app.platter.streamdeck.listen" })
class ListenAlong extends Watching {
  protected view = plainView;
  override onKeyDown(ev: KeyDownEvent): Promise<void> {
    return run({ command: "listenAlong" }, ev.action, true);
  }
}

@action({ UUID: "app.platter.streamdeck.volume" })
class Volume extends SingletonAction {
  private stop: (() => void) | null = null;

  override onWillAppear(_ev: WillAppearEvent): void {
    this.stop ??= poller.subscribe((s, err) => {
      const f = volumeFeedback(s, err);
      for (const a of this.actions) if (a.isDial()) void (a as DialAction).setFeedback(f);
    });
    void poller.refresh();
  }

  override onWillDisappear(_ev: WillDisappearEvent): void {
    if ([...this.actions].length <= 1) {
      this.stop?.();
      this.stop = null;
    }
  }

  override onDialRotate(ev: DialRotateEvent): Promise<void> {
    return run({ command: "volumeStep", value: ev.payload.ticks * DIAL_STEP }, ev.action);
  }

  override onDialDown(ev: DialDownEvent): Promise<void> {
    return run({ command: "playPause" }, ev.action);
  }

  override onTouchTap(ev: TouchTapEvent): Promise<void> {
    return run({ command: "playPause" }, ev.action);
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

for (const a of [new PlayPause(), new Next(), new Previous(), new Like(), new Volume(), new RequestAlbum(), new ListenAlong()]) {
  streamDeck.actions.registerAction(a);
}

void streamDeck.connect().then(async () => {
  const g = await streamDeck.settings.getGlobalSettings<Globals>();
  client.setToken(g.token ?? null);
  // First connect with no token: ask Platter to pair.
  pairer.start(g.token ? null : (g.request ?? null));
  await poller.refresh();
});
