/**
 * Settings > Bridge: Platter Bridge, the optional helper for a server
 * (docs/BRIDGE.md), as one card. It says what the Bridge does for you, in
 * order, with the things it does today first-class and the rest marked
 * as coming. Not paired, the card leads with the three steps (copy one
 * line, start it, type the code it shows). Paired, it says how each thing
 * is going. Each benefit's switch, the night hours and the other ways to
 * pair sit under the page's More (rows in SettingsPage).
 */
import { useCallback, useEffect, useState, type ReactNode } from "react";
import { Button } from "../components/Button";
import { Skeleton } from "../components/Skeleton";
import { Tabs } from "../components/Tabs";
import { backend } from "../lib/backend";
import { timeAgo } from "../lib/relativeTime";
import type { BridgeEnvSetting, BridgeInfo, BridgeLyricsLine, BridgeMusicAccess, BridgeNightHours, BridgeOverview, Settings } from "../lib/types";
import { toast } from "../stores/toasts";
import { useSettings } from "../stores/settings";
import "./feed-settings.css";
import "./bridge-settings.css";
import { failText } from "../lib/errorText";
import { compareSemver } from "../lib/version";

export const BRIDGE_DESCRIPTION =
  "Optional. A small helper for your server that does Platter's heavy and always-on work while your computers are off: lyrics for every song, transcribed at night, and Lidarr from anywhere. Platter works fully without it, and nothing is installed on your server unless you set it up. It talks to Platter only through the same sealed relay as Listen along, and opens no ports.";

const SECTION_WORDS = "bridge server docker unraid container home side lidarr pair code add unpair headless relay lyrics transcribe night";

/** What the Bridge does for you, in this order. `live`: it does it today. */
export const BENEFITS = [
  { key: "lyrics", title: "Lyrics for every song", text: "Songs with no lyrics are transcribed at night on the server, for every device. Your computer never does the heavy work.", live: true },
  { key: "sound", title: "Sound analysis on the server", text: "", live: false },
  { key: "downloads", title: "Downloads ready when you leave", text: "", live: false },
  { key: "lidarr", title: "Lidarr from anywhere", text: "Requests and catch-up keep going, even with every computer off.", live: true },
  { key: "history", title: "One listening history", text: "", live: false },
] as const;

export type BenefitKey = (typeof BENEFITS)[number]["key"];

/** Step 1: the one line for the server, and the Unraid template. */
export const DOCKER_LINE =
  "docker run -d --name platter-bridge --restart unless-stopped --user 1000:1000 -v /path/to/appdata:/data -v /path/to/music:/music:ro ghcr.io/silvercyclops999/platter-bridge:latest";
export const UNRAID_TEMPLATE = "https://raw.githubusercontent.com/Silvercyclops999/platter/main/docker/bridge/platter-bridge.xml";

export const NIGHT_HOURS_OPTIONS: { value: BridgeNightHours; label: string }[] = [
  { value: "late", label: "11 PM to 6 AM" },
  { value: "night", label: "1 AM to 7 AM" },
  { value: "early", label: "2 AM to 8 AM" },
  { value: "always", label: "Any time" },
];

const ENV_WORDS: Record<BridgeEnvSetting, string> = {
  lidarrUrl: "Lidarr's address",
  lidarrKey: "Lidarr's key",
  plexUrl: "Plex's address",
  plexToken: "the Plex token",
  musicPlexPath: "the music folder's Plex path",
};

/** "Lidarr's address and Lidarr's key" */
export function envList(keys: BridgeEnvSetting[]): string {
  const words = keys.map((k) => ENV_WORDS[k]).filter(Boolean);
  if (words.length < 2) return words.join("");
  return `${words.slice(0, -1).join(", ")} and ${words[words.length - 1]}`;
}

const n = (x: number) => x.toLocaleString("en-US");

function hoursText(h: BridgeNightHours): string {
  return h === "always" ? "any time the server isn't busy" : (NIGHT_HOURS_OPTIONS.find((o) => o.value === h)?.label ?? "at night");
}

/** The Bridge's lyrics work in a line, for its card. */
export function lyricsWords(line: BridgeLyricsLine | null | undefined, on: boolean): { text: string; problem: boolean } {
  if (!on) return { text: "Off.", problem: false };
  if (!line) return { text: "Needs a newer Bridge. Update it on your server to get this.", problem: false };
  const tally = `${n(line.done)} ${line.done === 1 ? "song" : "songs"} done, about ${n(line.left)} to go`;
  switch (line.state) {
    case "off":
      return { text: "Starting.", problem: false };
    case "unavailable":
    case "noMusic":
    case "problem":
      return { text: line.problem ?? "Something needs looking at on the Bridge.", problem: true };
    case "downloading":
      return { text: "Downloading the speech model. It does this once.", problem: false };
    case "waiting":
      return { text: `Works ${hoursText(line.hours)}. ${tally}.`, problem: false };
    case "busy":
      return { text: `Waiting while your server is busy. ${tally}.`, problem: false };
    case "working":
      return { text: `${line.song ? `Transcribing ${line.song}. ` : "Working now. "}${tally}.`, problem: false };
    case "done":
      return { text: `Every song has lyrics. ${n(line.done)} done on the Bridge.`, problem: false };
  }
}

/** The Bridge's lyrics work is going, so this computer leaves its background batch to it. */
export function takesOver(b: BridgeInfo, on: boolean): boolean {
  const s = b.status?.lyrics?.state;
  return on && b.online && (s === "downloading" || s === "waiting" || s === "busy" || s === "working" || s === "done");
}

/** Step 3, and Add another Bridge under More: enter the code a Bridge shows in its log. */
export function AddBridge({ onAdded, compact = false }: { onAdded?: () => void; compact?: boolean }) {
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const add = () => {
    if (!code.trim() || busy) return;
    setBusy(true);
    setError(null);
    backend.bridgeJoin(code.trim()).then(
      (name) => {
        setBusy(false);
        setCode("");
        toast.info(`Paired with ${name}`);
        onAdded?.();
      },
      (e: unknown) => {
        setBusy(false);
        setError(failText("Couldn't add the Bridge", e, 300));
      },
    );
  };
  return (
    <div className={compact ? "bridge-add" : "bridge-add bridge-add--row"} data-testid="bridge-add">
      <div className="bridge-add__field">
        <input
          className="setting-text bridge-add__input"
          value={code}
          placeholder="PBB-..."
          spellCheck={false}
          autoCapitalize="off"
          autoComplete="off"
          aria-label="The code from the Bridge, starting with PBB-"
          onChange={(e) => setCode(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              add();
            }
          }}
        />
        <Button size="sm" variant="primary" disabled={busy || !code.trim()} onClick={add}>
          {busy ? "Waiting for the Bridge" : "Add the Bridge"}
        </Button>
      </div>
      {error && (
        <p className="info-key__error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}

export function bridgeMatches(query: string): boolean {
  const words = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
  const hay = `${SECTION_WORDS} ${BRIDGE_DESCRIPTION} ${BENEFITS.map((b) => b.title).join(" ")}`.toLowerCase();
  return words.every((w) => hay.includes(w));
}

const MUSIC: Record<BridgeMusicAccess, string> = {
  readWrite: "Music folder mounted read-write.",
  readOnly: "Music folder mounted read-only.",
  none: "No music folder mounted.",
};

export const BRIDGE_UPDATE_DOCS = "https://github.com/Silvercyclops999/platter/blob/main/docs/BRIDGE.md#updates";

/** When the Bridge and this app aren't the same version: what to do about it. Null when equal or unknown. */
export function bridgeVersionLine(bridge: string | null | undefined, app: string | null | undefined): { behind: boolean; text: string } | null {
  const c = compareSemver(bridge, app);
  if (c === -1)
    return {
      behind: true,
      text: `Platter Bridge ${bridge} is behind this app (${app}). Update it on your server: on Unraid the Docker tab shows the update, or the Auto Update Applications plugin does it for you.`,
    };
  if (c === 1) return { behind: false, text: "Platter Bridge is newer than this app; update Platter." };
  return null;
}

/** This app's version, once. */
function useAppVersion(): string | null {
  const [v, setV] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    backend.appInfo().then(
      (i) => {
        if (live) setV(i.version);
      },
      () => {},
    );
    return () => {
      live = false;
    };
  }, []);
  return v;
}

/** Kept current from `bridge://changed`. */
export function useBridge(): [BridgeOverview | null, (o: BridgeOverview) => void, () => void] {
  const [overview, setOverview] = useState<BridgeOverview | null>(null);
  const load = useCallback(() => {
    backend.bridgeOverview().then(setOverview, () => {});
  }, []);
  useEffect(() => {
    load();
    const un = backend.listen("bridge://changed", load);
    return () => void un.then((f) => f());
  }, [load]);
  return [overview, setOverview, load];
}

function copy(text: string, what: string) {
  void Promise.resolve(navigator.clipboard?.writeText(text)).then(
    () => toast.info(`${what} copied`),
    () => toast.error("Couldn't copy. Select the text and copy it instead"),
  );
}

/** One benefit: its name, and what it's doing (or what it does). */
function Benefit({ title, live, children }: { title: string; live: boolean; children?: ReactNode }) {
  return (
    <li className={`bridge-benefit${live ? "" : " is-coming"}`}>
      <span className="bridge-benefit__title">{title}</span>
      {live ? children && <span className="bridge-benefit__text">{children}</span> : <span className="bridge-benefit__soon">Coming</span>}
    </li>
  );
}

/** Not paired: what it does, then the three steps. */
function SetUp({ onAdded }: { onAdded: () => void }) {
  const [where, setWhere] = useState<"docker" | "unraid">("docker");
  return (
    <div className="bridge-card" data-testid="bridge-setup">
      <div className="bridge-card__head">
        <h3 className="bridge-card__title">Platter Bridge</h3>
        <p className="bridge-card__lead">A small helper for your server. It does the heavy work while your computers are off. Optional: Platter works fully without it.</p>
      </div>
      <ol className="bridge-benefits" aria-label="What it does for you">
        {BENEFITS.map((b) => (
          <Benefit key={b.key} title={b.title} live={b.live}>
            {b.text}
          </Benefit>
        ))}
      </ol>
      <h4 className="bridge-card__subtitle">Set it up in three steps</h4>
      <ol className="bridge-steps">
        <li className="bridge-step">
          <span className="bridge-step__n" aria-hidden="true">
            1
          </span>
          <div className="bridge-step__body">
            <div className="bridge-step__title">Copy one line onto your server</div>
            <Tabs
              label="Where it runs"
              idPrefix="bridge-where"
              className="bridge-step__tabs"
              items={[
                { id: "docker", label: "Docker" },
                { id: "unraid", label: "Unraid" },
              ]}
              value={where}
              onChange={setWhere}
            />
            <div id="bridge-where-panel" role="tabpanel" aria-labelledby={`bridge-where-tab-${where}`}>
              {where === "docker" ? (
                <>
                  <code className="bridge-line" data-testid="bridge-line">
                    {DOCKER_LINE}
                  </code>
                  <p className="bridge-step__hint">Change the two folders to yours. The music folder is for lyrics; read-only is enough.</p>
                  <Button size="sm" onClick={() => copy(DOCKER_LINE, "The line")}>
                    Copy the line
                  </Button>
                </>
              ) : (
                <>
                  <code className="bridge-line">{UNRAID_TEMPLATE}</code>
                  <p className="bridge-step__hint">In Unraid's Docker tab, add a container from this template, and pick your music share as its Music folder.</p>
                  <Button size="sm" onClick={() => copy(UNRAID_TEMPLATE, "The template link")}>
                    Copy the template link
                  </Button>
                </>
              )}
            </div>
          </div>
        </li>
        <li className="bridge-step">
          <span className="bridge-step__n" aria-hidden="true">
            2
          </span>
          <div className="bridge-step__body">
            <div className="bridge-step__title">Start it</div>
            <p className="bridge-step__hint">It shows a code starting with PBB- in its log. It works once, for an hour.</p>
          </div>
        </li>
        <li className="bridge-step">
          <span className="bridge-step__n" aria-hidden="true">
            3
          </span>
          <div className="bridge-step__body">
            <div className="bridge-step__title">Type the code it shows</div>
            <AddBridge onAdded={onAdded} compact />
          </div>
        </li>
      </ol>
    </div>
  );
}

/** Paired: how each thing is going. */
function PairedCard({ b, appVersion, settings, onUnpair }: { b: BridgeInfo; appVersion: string | null; settings: Settings | null; onUnpair: () => void }) {
  const [sure, setSure] = useState(false);
  const st = b.status;
  const last = st?.lastCatchUp;
  const ver = bridgeVersionLine(st?.version || b.version, appVersion);
  const lyricsOn = settings?.bridgeLyrics ?? true;
  const lidarrOn = settings?.bridgeLidarr ?? true;
  const lyrics = lyricsWords(st?.lyrics, lyricsOn);
  const lidarrText = !lidarrOn
    ? "Off."
    : last
      ? last.error
        ? `Last catch-up ${timeAgo(last.at).toLowerCase()} didn't finish: ${last.error}`
        : `Last catch-up ${timeAgo(last.at).toLowerCase()}: ${last.handled} Lidarr ${last.handled === 1 ? "event" : "events"}, ${last.acted} acted on${last.announced.length ? `, new: ${last.announced.join(", ")}` : ""}.`
      : "No catch-up yet.";
  return (
    <div className="bridge-card" data-testid="bridge-row">
      <div className="bridge-card__head bridge-card__head--paired">
        <div className="bridge-card__name">
          <h3 className="bridge-card__title">{b.name}</h3>
          <span className={`settings-card__status${b.online ? " is-on" : ""}`}>{b.online ? "Answering" : "Not heard from lately"}</span>
        </div>
        <p className="bridge-card__lead">
          {b.lastSeen ? `Last heard: ${timeAgo(b.lastSeen).toLowerCase()}. ` : ""}
          {MUSIC[b.music]}
        </p>
        {ver && (
          <p className="bridge-card__lead" data-testid="bridge-version">
            {ver.text}
            {ver.behind && (
              <>
                {" "}
                <button
                  type="button"
                  className="link bridge-version__link"
                  onClick={() => void backend.helpOpenLink(BRIDGE_UPDATE_DOCS).catch(() => toast.error("Couldn't open the browser"))}
                >
                  How to update
                </button>
              </>
            )}
          </p>
        )}
      </div>
      <ol className="bridge-benefits" aria-label="What it does for you">
        {BENEFITS.map((x) =>
          x.key === "lyrics" ? (
            <Benefit key={x.key} title={x.title} live>
              <span data-testid="bridge-lyrics" className={lyrics.problem ? "bridge-benefit__problem" : undefined}>
                {lyrics.text}
                {takesOver(b, lyricsOn) && " This computer leaves background transcribing to it."}
              </span>
            </Benefit>
          ) : x.key === "lidarr" ? (
            <Benefit key={x.key} title={x.title} live>
              {lidarrText}
            </Benefit>
          ) : (
            <Benefit key={x.key} title={x.title} live={false} />
          ),
        )}
      </ol>
      {st && <p className="bridge-card__now">Now: {st.doing}</p>}
      {st && st.fromEnv.length > 0 && (
        <p className="bridge-card__note" data-testid="bridge-env">
          Set on the Bridge itself: {envList(st.fromEnv)}. It uses those, not what this computer sends.
        </p>
      )}
      {st?.plexLink && (
        <p className="bridge-card__note" data-testid="bridge-plex-link">
          To let it read your Plex library, open plex.tv/link and enter <code>{st.plexLink}</code>.
        </p>
      )}
      {st?.musicPlexPath && (
        <p className="bridge-card__note" data-testid="bridge-music-path">
          Its music folder matches Plex's {st.musicPlexPath}. Set MUSIC_PLEX_PATH to <code>{st.musicPlexPath}</code> on the Bridge so it can add albums itself.
        </p>
      )}
      {st?.problem && (
        <p className="info-key__error" role="alert">
          {st.problem}
        </p>
      )}
      <div className="bridge-card__actions">
        {sure ? (
          <span>
            <Button size="sm" variant="primary" onClick={onUnpair}>
              Unpair {b.name}
            </Button>{" "}
            <Button size="sm" variant="ghost" onClick={() => setSure(false)}>
              Cancel
            </Button>
          </span>
        ) : (
          <Button size="sm" variant="ghost" onClick={() => setSure(true)}>
            Unpair
          </Button>
        )}
      </div>
    </div>
  );
}

/** Under More: start the Bridge with a code made here (BRIDGE_PAIR_CODE). */
export function PairWithCode() {
  const [ov, , load] = useBridge();
  const pair = () =>
    backend.bridgePairStart().then(
      () => load(),
      () => toast.error("Couldn't make a pair code"),
    );
  const cancel = () => void backend.bridgePairCancel().then(load);
  if (!ov) return <Skeleton width={120} height={32} radius="pill" />;
  if (!ov.pairing)
    return (
      <Button size="sm" onClick={() => void pair()}>
        Pair a Bridge
      </Button>
    );
  return (
    <div data-testid="bridge-code" className="bridge-code-block">
      <p className="setting-row__desc">Start the Bridge with BRIDGE_PAIR_CODE set to this code. It works once, for the next hour, while Platter is open here:</p>
      <code className="bridge-code">{ov.pairing.code}</code>{" "}
      <Button size="sm" variant="ghost" onClick={() => copy(ov.pairing?.code ?? "", "The code")}>
        Copy
      </Button>{" "}
      <Button size="sm" variant="ghost" onClick={cancel}>
        Cancel
      </Button>
    </div>
  );
}

export function BridgeSettings() {
  const [ov, setOv, load] = useBridge();
  const appVersion = useAppVersion();
  const settings = useSettings((x) => x.settings);
  const unpair = (b: BridgeInfo) =>
    backend.bridgeUnpair(b.id).then(
      (o) => {
        setOv(o);
        toast.info(`${b.name} was unpaired. Remove its container if you don't need it any more`);
      },
      () => toast.error("Couldn't unpair the Bridge"),
    );
  const answer = (id: string, yes: boolean) =>
    (yes ? backend.bridgeQueuedApply(id) : backend.bridgeQueuedDismiss(id)).then(setOv, (e: unknown) =>
      toast.error(failText(yes ? "Couldn't apply the Bridge's queued change" : "Couldn't dismiss the Bridge's queued change", e)),
    );
  return (
    <section className="bridge-settings" aria-label="Platter Bridge" data-testid="bridge-settings">
      {!ov ? (
        <Skeleton height={320} radius="card" />
      ) : ov.bridges.length === 0 ? (
        <SetUp onAdded={load} />
      ) : (
        ov.bridges.map((b) => <PairedCard key={b.id} b={b} appVersion={appVersion} settings={settings} onUnpair={() => void unpair(b)} />)
      )}
      {ov?.queued.map((q) => (
        <div className="setting-row" key={q.id} data-testid="bridge-queued">
          <div className="setting-row__text">
            <div className="setting-row__title">Waiting for you: {q.what}</div>
            <p className="setting-row__desc">
              {q.device} didn't make this change to Lidarr's settings, because a Bridge never does that on its own. Exactly what it would send:{" "}
              <code style={{ wordBreak: "break-all" }}>
                {q.method} {q.path} {q.body}
              </code>
            </p>
          </div>
          <span>
            <Button size="sm" onClick={() => void answer(q.id, true)}>
              Make this change
            </Button>{" "}
            <Button size="sm" variant="ghost" onClick={() => void answer(q.id, false)}>
              Dismiss
            </Button>
          </span>
        </div>
      ))}
    </section>
  );
}
