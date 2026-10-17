/**
 * The setup guides: short, skippable, resumable steps that end on something
 * working, then say what's next. Each step reuses the same controls as
 * Settings, so a guide and Settings never disagree.
 */
import { useId, useState, type ReactNode } from "react";
import { Button } from "../components/Button";
import { Switch } from "../components/Switch";
import type { Settings } from "../lib/types";
import { IMPORT_SOURCES } from "../lib/importStrings";
import type { ImportReport, ImportSourceId } from "../lib/importTypes";
import { useApp } from "../stores/app";
import { help } from "../stores/help";
import { nav } from "../stores/router";
import { updateSettings, useSettings } from "../stores/settings";
import { openInvite, startSession, useSocial } from "../stores/social";
import { GUIDE_TITLES, type GuideId } from "../stores/guides";
import { LibrarySourceList } from "../pages/LibrarySourceSettings";
import { ThemeSelect } from "../pages/ThemeSettings";
import { LookCards } from "../pages/looks/LookCards";
import { discordNote, useDiscordStatus } from "../pages/DiscordSettings";
import { LidarrCheckButton, LidarrFindings, LidarrKeyControl, LidarrUrlField, useLidarr } from "../pages/LidarrSettings";
import { StagingFolderField } from "../pages/LidarrUpgrade";
import { importFrom, reportLine, useImportOverview } from "../pages/importShared";
import { ImportHistoryLinks } from "../pages/ImportHistoryLinks";
import { DeckPairing, DeckPluginRow, useDeckPlugin, useDeckStatus } from "../pages/StreamDeckSettings";
import type { HelpRef } from "../pages/settingsMeta";

export interface GuideStep {
  id: string;
  title: string;
  /** What to do, in a sentence or two. */
  text: string;
  /** The working part (a picker, a switch, a check). */
  Body?: () => ReactNode;
  /** "More about this" in the help panel. */
  help?: HelpRef;
}

export interface NextThing {
  label: string;
  run: () => void;
}

export interface Guide {
  id: GuideId;
  title: string;
  /** One line for the list of guides in Settings. */
  summary: string;
  steps: GuideStep[];
  /** The last screen: what's working now. */
  doneText: string;
  next: NextThing[];
}

// ------------------------------------------------------------------ small parts

/** A switch with its label, for a guide step. */
function SwitchLine({ label, checked, onChange }: { label: string; checked: boolean; onChange: (v: boolean) => void }) {
  const id = useId();
  return (
    <div className="guide-switch">
      <span id={id}>{label}</span>
      <Switch checked={checked} onChange={onChange} labelledBy={id} />
    </div>
  );
}

const ids = (id: string) => ({ title: `${id}-t`, desc: `${id}-d` });

function Labelled({ label, children }: { label: string; children: (i: { title: string; desc: string }) => ReactNode }) {
  const id = useId();
  return (
    <div className="guide-field">
      <span id={`${id}-t`} className="guide-field__label">
        {label}
      </span>
      <span id={`${id}-d`} hidden />
      {children(ids(id))}
    </div>
  );
}

// ------------------------------------------------------------------ first run

function SignedIn() {
  const auth = useApp((x) => x.auth);
  return (
    <p className="guide-state" role="status">
      {auth?.username ? `You're signed in as ${auth.username}${auth.server ? ` on ${auth.server.name}` : ""}.` : "Sign in with Plex to go on."}
    </p>
  );
}

function PickLibraries() {
  const s = useSettings((x) => x.settings);
  return <LibrarySourceList settings={s} />;
}

function PickTheme() {
  const s = useSettings((x) => x.settings);
  if (!s) return null;
  return (
    <>
      <LookCards />
      <Labelled label="Light or dark">{(i) => <ThemeSelect s={s} ids={i} />}</Labelled>
    </>
  );
}

function DiscordStep() {
  const s = useSettings((x) => x.settings);
  const { status } = useDiscordStatus();
  const note = discordNote(status, s?.discordStatus);
  return (
    <>
      <SwitchLine label="Show what I'm playing on Discord" checked={!!s?.discordStatus} onChange={(v) => void updateSettings({ discordStatus: v })} />
      {note && <p className="guide-state">{note}</p>}
    </>
  );
}

/** Look-ups that ask other services about artists and releases (off on a new install). */
export const LOOKUPS: (keyof Settings)[] = [
  "infoWikipedia",
  "infoMusicbrainz",
  "infoListenbrainz",
  "infoListenbrainzStats",
  "catalogueSearch",
  "fansAlsoLikeCatalogue",
  "homeNewToYou",
  "releaseRadar",
];

function LookupsStep() {
  const s = useSettings((x) => x.settings);
  const on = !!s && LOOKUPS.every((k) => s[k] === true);
  const set = (v: boolean) => void updateSettings(Object.fromEntries(LOOKUPS.map((k) => [k, v])) as Partial<Settings>);
  return <SwitchLine label="Look up artist info and new releases online" checked={on} onChange={set} />;
}

// ------------------------------------------------------------------ Lidarr

function LidarrAddress() {
  return <Labelled label="Lidarr's address">{(i) => <LidarrUrlField field="lidarrUrl" placeholder="http://192.168.x.x:8686" ids={i} />}</Labelled>;
}

function LidarrKey() {
  const lidarr = useLidarr();
  return <Labelled label="Lidarr's key">{(i) => <LidarrKeyControl lidarr={lidarr} ids={i} />}</Labelled>;
}

function LidarrCheck() {
  const lidarr = useLidarr();
  return (
    <>
      <LidarrCheckButton lidarr={lidarr} />
      <LidarrFindings lidarr={lidarr} />
    </>
  );
}

function LidarrStaging() {
  return <Labelled label="Staging folder">{(i) => <StagingFolderField ids={i} />}</Labelled>;
}

// ------------------------------------------------------------------ library import

function ImportPick() {
  const s = useSettings((x) => x.settings);
  const enabled = s?.importSources ?? [];
  return (
    <>
      {IMPORT_SOURCES.map((src) => (
        <SwitchLine
          key={src.id}
          label={src.name}
          checked={enabled.includes(src.id)}
          onChange={(v) => void updateSettings({ importSources: v ? [...enabled, src.id] : enabled.filter((x) => x !== src.id) })}
        />
      ))}
    </>
  );
}

function ImportGetFile() {
  const s = useSettings((x) => x.settings);
  const enabled = IMPORT_SOURCES.filter((src) => s?.importSources.includes(src.id));
  if (!enabled.length) return <p className="guide-state">Go back a step and pick where your music is.</p>;
  return (
    <ul className="guide-list">
      {enabled.map((src) => (
        <li key={src.id}>
          <strong>{src.name}</strong>: {src.help}
        </li>
      ))}
    </ul>
  );
}

function ImportChoose() {
  const s = useSettings((x) => x.settings);
  const [busy, setBusy] = useState(false);
  const [last, setLast] = useState<ImportReport | null>(null);
  const enabled = IMPORT_SOURCES.filter((src) => s?.importSources.includes(src.id));
  const pick = async (id: ImportSourceId, folder: boolean) => {
    setBusy(true);
    const r = await importFrom(id, folder);
    setBusy(false);
    if (r) setLast(r);
  };
  return (
    <>
      {enabled.map((src) => (
        <div key={src.id} className="guide-actions">
          <span className="guide-field__label">{src.name}</span>
          <Button size="sm" disabled={busy} onClick={() => void pick(src.id, false)}>
            Choose file
          </Button>
          <Button size="sm" variant="ghost" disabled={busy} onClick={() => void pick(src.id, true)}>
            Choose folder
          </Button>
        </div>
      ))}
      {busy && <p className="guide-state">Importing and matching to your library…</p>}
      {last && (
        <>
          <p className="guide-state" role="status">
            {reportLine(last)}
          </p>
          <ImportHistoryLinks r={last} />
        </>
      )}
    </>
  );
}

function ImportReportStep() {
  const [overview] = useImportOverview();
  const n = overview?.reports.length ?? 0;
  return (
    <p className="guide-state" role="status">
      {n ? `${n === 1 ? "One import is" : `${n} imports are`} in. ${overview?.reviewCount ? `${overview.reviewCount} matches want a quick look.` : ""}` : "Nothing imported yet. You can go back a step, or come back later."}
    </p>
  );
}

// ------------------------------------------------------------------ sharing

function StartListening() {
  const role = useSocial((x) => x.listen.role);
  return role === "host" ? (
    <p className="guide-state" role="status">
      Your listening session is on.
    </p>
  ) : (
    <Button size="sm" variant="primary" onClick={() => void startSession()}>
      Start a listening session
    </Button>
  );
}

function InviteFriend() {
  const role = useSocial((x) => x.listen.role);
  if (role !== "host") return <p className="guide-state">Start a session first (the step before this one).</p>;
  return (
    <Button size="sm" variant="primary" onClick={() => void openInvite()}>
      Show the invite link
    </Button>
  );
}

// ------------------------------------------------------------------ Stream Deck

function DeckInstall() {
  const [plugin, refresh] = useDeckPlugin();
  return <DeckPluginRow status={plugin} onChanged={refresh} />;
}

function DeckAllow() {
  const s = useSettings((x) => x.settings);
  const [status] = useDeckStatus();
  return (
    <>
      <SwitchLine label="Allow Stream Deck" checked={!!s?.allowStreamDeck} onChange={(v) => void updateSettings({ allowStreamDeck: v })} />
      {s?.allowStreamDeck && <DeckPairing status={status} onRevoke={() => {}} />}
    </>
  );
}

// ------------------------------------------------------------------ the guides

export const GUIDES: Guide[] = [
  {
    id: "firstRun",
    title: GUIDE_TITLES.firstRun,
    summary: "Sign in, pick your music, choose a look, and decide what Platter may share.",
    steps: [
      { id: "signIn", title: "Sign in", text: "Platter plays the music on your Plex server, so it needs your Plex sign-in.", Body: SignedIn, help: ["getting-started", "signing-in-with-plex"] },
      { id: "libraries", title: "Pick your music", text: "Choose which music libraries Platter shows. You can change this any time in Settings.", Body: PickLibraries, help: ["getting-started", "picking-libraries"] },
      { id: "theme", title: "Choose a look", text: "Pick a ready-made look, then light or dark. Settings has more, and you can make your own.", Body: PickTheme, help: ["getting-started", "picking-a-look"] },
      { id: "discord", title: "Discord", text: "Discord can show friends what you're playing. It's off until you turn it on.", Body: DiscordStep, help: ["getting-started", "discord"] },
      {
        id: "lookups",
        title: "Artist info and new releases",
        text: "Platter can look artists up on Wikipedia, MusicBrainz and ListenBrainz for bios, popular songs and new releases. Only artist names and ids are sent. It's off until you turn it on.",
        Body: LookupsStep,
        help: ["privacy", "off-until-you-turn-them-on"],
      },
    ],
    doneText: "You're all set. Your music is ready to play.",
    next: [
      { label: "Go to Home", run: () => nav.go({ name: "home" }) },
      { label: "Search your music", run: () => nav.go({ name: "search", query: "" }) },
      { label: "Read the help", run: () => help.open("getting-started") },
    ],
  },
  {
    id: "lidarr",
    title: GUIDE_TITLES.lidarr,
    summary: "Request missing albums and upgrades through your Lidarr, with a safety check first.",
    steps: [
      { id: "address", title: "Lidarr's address", text: "Type the address you open Lidarr with in your browser, like http://192.168.x.x:8686.", Body: LidarrAddress, help: ["lidarr", "connecting-lidarr"] },
      { id: "key", title: "Lidarr's key", text: "In Lidarr, open Settings, General and copy the API Key. Platter keeps it in this computer's keychain.", Body: LidarrKey, help: ["lidarr", "connecting-lidarr"] },
      {
        id: "check",
        title: "The safety check",
        text: "Platter reads Lidarr's setup and makes sure it can't touch the music you look after yourself. Red items must be fixed; amber ones are worth a look. It never changes anything in Lidarr during the check.",
        Body: LidarrCheck,
        help: ["lidarr", "the-safety-check"],
      },
      {
        id: "modes",
        title: "Managed and Protected artists",
        text: "Managed artists are Lidarr's to look after: it imports, renames and upgrades in their folders. Protected artists have files you curate: Lidarr only downloads into the staging folder, and Platter adds albums to your library itself. Artists with unreleased or demo albums start Protected. You can change any artist under Managed and protected artists.",
        help: ["lidarr", "managed-and-protected-artists"],
      },
      {
        id: "staging",
        title: "The staging folder",
        text: "Upgrades land in a staging folder first: one of Lidarr's root folders that Plex doesn't scan. You compare the new copy, and only then is it swapped in. Leave it empty to keep upgrades off.",
        Body: LidarrStaging,
        help: ["lidarr", "the-staging-folder"],
      },
    ],
    doneText: "Lidarr is connected. For the first week Platter runs in practice mode: it shows what it would have done without doing it.",
    next: [
      { label: "Read about practice mode", run: () => help.open("lidarr", "practice-mode") },
      { label: "Open Lidarr settings", run: () => nav.go({ name: "settings" }) },
    ],
  },
  {
    id: "import",
    title: GUIDE_TITLES.import,
    summary: "Bring your playlists, likes and history from another service.",
    steps: [
      { id: "service", title: "Where's your music now?", text: "Turn on the service you're coming from. Nothing is read until you pick a file.", Body: ImportPick, help: ["library-import", "which-services"] },
      { id: "file", title: "Get the file", text: "Each service has its own way to give you a copy of your library.", Body: ImportGetFile, help: ["library-import", "which-services"] },
      { id: "choose", title: "Pick the file", text: "Choose the file or the folder you downloaded. Platter matches it to your library on this computer.", Body: ImportChoose, help: ["library-import", "what-an-import-does"] },
      { id: "report", title: "The report", text: "The report shows what matched, what wants a quick look, and what's missing from your library.", Body: ImportReportStep, help: ["library-import", "the-import-report"] },
    ],
    doneText: "Your import is in. Imported playlists are in your library.",
    next: [
      { label: "See the report", run: () => nav.go({ name: "import" }) },
      { label: "See what's missing", run: () => nav.go({ name: "import", view: "missing" }) },
    ],
  },
  {
    id: "sharing",
    title: GUIDE_TITLES.sharing,
    summary: "Listen together: start a session and send a friend the invite.",
    steps: [
      {
        id: "what",
        title: "Listening together",
        text: "In a listening session, friends hear what you play, in time with you, in their own Platter. They play each song from your Plex library when they have access to it, or from their own copy.",
        help: ["listening-together", "start-a-listening-session"],
      },
      { id: "start", title: "Start a session", text: "Start one now, or later from a song, album or playlist menu.", Body: StartListening, help: ["listening-together", "start-a-listening-session"] },
      { id: "invite", title: "Send the invite", text: "Copy the link and send it to your friend. Each link works for one person, once, and runs out after a while.", Body: InviteFriend, help: ["listening-together", "invite-links-and-qr-codes"] },
    ],
    doneText: "Your session is on. People who join show in the bar at the bottom.",
    next: [
      { label: "What guests can do", run: () => help.open("listening-together", "what-guests-hear-and-do") },
      { label: "Session settings", run: () => nav.go({ name: "settings" }) },
    ],
  },
  {
    id: "streamDeck",
    title: GUIDE_TITLES.streamDeck,
    summary: "Play, skip and like from an Elgato Stream Deck.",
    steps: [
      { id: "install", title: "Install the plugin", text: "Platter's plugin comes with Platter. Install it here and the Stream Deck app finishes the job. Without the Stream Deck app, this links to Elgato's download page.", Body: DeckInstall, help: ["stream-deck", "installing-the-plugin"] },
      { id: "allow", title: "Allow it", text: "Allow Stream Deck lets the plugin control Platter, on this computer only. Installing turns it on. The first time you press a Platter key, Platter asks once; choose Allow.", Body: DeckAllow, help: ["stream-deck", "allowing-it-in-platter"] },
      { id: "keys", title: "Drag the keys", text: "In the Stream Deck app, find Platter in the list of actions and drag the keys you want onto your Stream Deck.", help: ["stream-deck", "the-keys"] },
    ],
    doneText: "Your Stream Deck controls Platter now.",
    next: [
      { label: "All the keys", run: () => help.open("stream-deck", "the-keys") },
      { label: "When a key doesn't work", run: () => help.open("stream-deck", "when-a-key-doesnt-work") },
    ],
  },
];

export function guideById(id: GuideId): Guide {
  return GUIDES.find((g) => g.id === id)!;
}
