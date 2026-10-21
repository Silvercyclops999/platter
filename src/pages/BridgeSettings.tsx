/**
 * Settings > Bridge: the optional helper for a server (docs/BRIDGE.md), as
 * one card. With no Bridge yet, it says what the Bridge does and the two
 * steps (copy one line, start it and sign it in to Plex): every Platter on
 * the same Plex account then finds it by itself, with no code. With one,
 * it's just "Bridge", whether it's working, one line each for lyrics and
 * Lidarr, anything you need to do, and the rest under Details. Each
 * benefit's switch and the night hours sit under the page's More (rows in
 * SettingsPage).
 */
import { useEffect, useState, type ReactNode } from "react";
import { create } from "zustand";
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

const SECTION_WORDS = "bridge server docker unraid container home side lidarr plex sign in link add unpair headless relay lyrics transcribe night";

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
      text: "The Bridge is out of date. Update it on your server.",
    };
  if (c === 1) return { behind: false, text: "The Bridge is newer than this app; update Platter." };
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

/**
 * One overview for every part of the page (the card, the page's status and
 * its More rows), so pairing or unpairing anywhere updates all of them.
 */
export const useBridgeStore = create<{ overview: BridgeOverview | null; set: (o: BridgeOverview) => void; load: () => void }>((set) => ({
  overview: null,
  set: (overview) => set({ overview }),
  load: () => {
    backend.bridgeOverview().then(
      (overview) => set({ overview }),
      () => {},
    );
  },
}));

/** Components using the overview now; the event is heard while there are any. */
let users = 0;
let unlisten: Promise<() => void> | null = null;

/** Kept current from `bridge://changed`. */
export function useBridge(): [BridgeOverview | null, (o: BridgeOverview) => void, () => void] {
  const overview = useBridgeStore((s) => s.overview);
  const set = useBridgeStore((s) => s.set);
  const load = useBridgeStore((s) => s.load);
  useEffect(() => {
    users += 1;
    if (users === 1) unlisten = backend.listen("bridge://changed", () => useBridgeStore.getState().load());
    load();
    return () => {
      users -= 1;
      if (users === 0 && unlisten) {
        const un = unlisten;
        unlisten = null;
        void un.then((f) => f());
      }
    };
  }, [load]);
  return [overview, set, load];
}

function copy(text: string, what: string) {
  const failed = () => toast.error("Couldn't copy. Select the text and copy it instead");
  try {
    if (!navigator.clipboard?.writeText) {
      failed();
      return;
    }
    navigator.clipboard.writeText(text).then(() => toast.info(`${what} copied`), failed);
  } catch {
    failed();
  }
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

/** No Bridge yet: what it does, then the two steps. */
function SetUp() {
  const [where, setWhere] = useState<"docker" | "unraid">("docker");
  return (
    <div className="bridge-card" data-testid="bridge-setup">
      <div className="bridge-card__head">
        <h3 className="bridge-card__title">Bridge</h3>
        <p className="bridge-card__lead">A small helper for your server. It does the heavy work while your computers are off. Optional: Platter works fully without it.</p>
      </div>
      <ol className="bridge-benefits" aria-label="What it does for you">
        {BENEFITS.filter((b) => b.live).map((b) => (
          <Benefit key={b.key} title={b.title} live={b.live}>
            {b.text}
          </Benefit>
        ))}
      </ol>
      <h4 className="bridge-card__subtitle">Set it up in two steps</h4>
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
            <div className="bridge-step__title">Start it and sign it in to Plex</div>
            <p className="bridge-step__hint">
              Its log shows a short code. Open plex.tv/link and enter it, signed in as the owner of your Plex server. Every Platter signed in as the owner finds the Bridge by itself within a minute. There's nothing to type here.
            </p>
          </div>
        </li>
      </ol>
    </div>
  );
}

/** Paired: how each thing is going. */
function PairedCard({ b, label, appVersion, settings, onUnpair }: { b: BridgeInfo; label?: string; appVersion: string | null; settings: Settings | null; onUnpair: () => void }) {
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
  const title = label ?? "Bridge";
  return (
    <div className="bridge-card" data-testid="bridge-row">
      <div className="bridge-card__head bridge-card__head--paired">
        <div className="bridge-card__name">
          <h3 className="bridge-card__title">{title}</h3>
          <span className={`settings-card__status${b.online ? " is-on" : ""}`}>{b.online ? "Working" : "Not answering"}</span>
        </div>
        {!b.online && b.lastSeen && <p className="bridge-card__lead">Last heard {timeAgo(b.lastSeen).toLowerCase()}.</p>}
      </div>
      <ul className="bridge-live" aria-label="What it does for you">
        <li className="bridge-live__item">
          <span className="bridge-live__title">Lyrics</span>
          <span data-testid="bridge-lyrics" className={lyrics.problem ? "bridge-benefit__problem" : "bridge-live__text"}>
            {lyrics.text}
            {takesOver(b, lyricsOn) && " This computer leaves background transcribing to it."}
          </span>
        </li>
        <li className="bridge-live__item">
          <span className="bridge-live__title">Lidarr</span>
          <span className="bridge-live__text">{lidarrText}</span>
        </li>
      </ul>
      {st?.plexLink && (
        <p className="bridge-card__note" data-testid="bridge-plex-link">
          To let it read your Plex library and find your devices, open plex.tv/link and enter <code>{st.plexLink}</code>.
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
      {ver?.behind && (
        <p className="bridge-card__note" data-testid="bridge-version">
          {ver.text}{" "}
          <button
            type="button"
            className="link bridge-version__link"
            onClick={() => void backend.helpOpenLink(BRIDGE_UPDATE_DOCS).catch(() => toast.error("Couldn't open the browser"))}
          >
            How to update
          </button>
        </p>
      )}
      <details className="bridge-card__details">
        <summary>Details</summary>
        {label && <p className="bridge-card__note">Its name on the server: {b.name}.</p>}
        {b.lastSeen && b.online && <p className="bridge-card__note">Last heard {timeAgo(b.lastSeen).toLowerCase()}.</p>}
        <p className="bridge-card__note">{MUSIC[b.music]}</p>
        {ver && !ver.behind && (
          <p className="bridge-card__note" data-testid="bridge-version">
            {ver.text}
          </p>
        )}
        {st && <p className="bridge-card__note">Now: {st.doing}</p>}
        {st && st.fromEnv.length > 0 && (
          <p className="bridge-card__note" data-testid="bridge-env">
            Set on the Bridge itself: {envList(st.fromEnv)}. It uses those, not what this computer sends.
          </p>
        )}
      </details>
      <div className="bridge-card__actions">
        {sure ? (
          <span>
            <Button size="sm" variant="primary" onClick={onUnpair}>
              Unpair {title}
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

/** What a Bridge is called here: just "Bridge", unless there's more than one. */
export function bridgeLabel(b: BridgeInfo, all: BridgeInfo[]): string | undefined {
  if (all.length < 2) return undefined;
  const same = all.filter((x) => x.name === b.name);
  return same.length < 2 ? b.name : `${b.name} ${same.indexOf(b) + 1}`;
}

export function BridgeSettings() {
  const [ov, setOv] = useBridge();
  const appVersion = useAppVersion();
  const settings = useSettings((x) => x.settings);
  const unpair = (b: BridgeInfo) =>
    backend.bridgeUnpair(b.id).then(
      (o) => {
        const name = (ov && bridgeLabel(b, ov.bridges)) ?? "The Bridge";
        setOv(o);
        toast.info(
          b.auto
            ? `${name} was unpaired. It won't link to this device again until one of them signs in to Plex again. Remove its container if you don't need it any more`
            : `${name} was unpaired. Remove its container if you don't need it any more`,
        );
      },
      () => toast.error("Couldn't unpair the Bridge"),
    );
  const answer = (id: string, yes: boolean) =>
    (yes ? backend.bridgeQueuedApply(id) : backend.bridgeQueuedDismiss(id)).then(setOv, (e: unknown) =>
      toast.error(failText(yes ? "Couldn't apply the Bridge's queued change" : "Couldn't dismiss the Bridge's queued change", e)),
    );
  return (
    <div className="bridge-settings" data-testid="bridge-settings">
      {!ov ? (
        <Skeleton height={320} radius="card" />
      ) : ov.bridges.length === 0 ? (
        <SetUp />
      ) : (
        ov.bridges.map((b) => <PairedCard key={b.id} b={b} label={bridgeLabel(b, ov.bridges)} appVersion={appVersion} settings={settings} onUnpair={() => void unpair(b)} />)
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
    </div>
  );
}
