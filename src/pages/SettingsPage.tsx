import { Server } from "lucide-react";
import { Fragment, useEffect, useId, useState, type ReactNode } from "react";
import { Button } from "../components/Button";
import { Dialog } from "../components/Dialog";
import { SearchField } from "../components/SearchField";
import { Select, type SelectOption } from "../components/Select";
import { Skeleton } from "../components/Skeleton";
import { Switch } from "../components/Switch";
import { useSidebarArt } from "../stores/ui";
import { backend } from "../lib/backend";
import { formatBytes } from "../lib/format";
import { modKey, platform } from "../lib/platform";
import type {
  AppError,
  AppInfo,
  ArtCacheInfo,
  ConnectionKind,
  PopularSource,
  ReduceMotion,
  ServerInfo,
  Settings,
  StreamQuality,
  UpdateInfo,
} from "../lib/types";
import { useApp } from "../stores/app";
import { NowPlayingSettings } from "../shell/NowPlayingSettings";
import { OutputDeviceSelect } from "../shell/OutputSetting";
import { reloadLikes } from "../stores/likes";
import { updateSettings, useSettings } from "../stores/settings";
import { PlaylistNameSetting } from "./PlaylistSettings";
import { InfoKeyControl, keyNote, useInfoKeys } from "./ArtistInfoSettings";
import { CrossfadeControl, EqEditor, PreampControl } from "./AudioSettings";
import { catboxNote, discordNote, ImgurKeyControl, imgurNote, useDiscordStatus } from "./DiscordSettings";
import { EXPLICIT_OPTIONS, TagScanNote } from "./FileTagSettings";
import { BatchControl, BatchNote, ModelControl, TranscribeNote } from "./TranscribeSettings";
import "./playlist.css";
import { toast } from "../stores/toasts";

const errText = (e: unknown) => (e && typeof e === "object" && "message" in e ? String((e as AppError).message) : "");

// ------------------------------------------------------------------ options

const QUALITY_LABEL: Record<StreamQuality, string> = {
  original: "Original",
  "320": "320 kbps",
  "256": "256 kbps",
  "192": "192 kbps",
  "128": "128 kbps",
};
const opts = <T extends string>(values: T[], label: Record<T, string>): SelectOption<T>[] =>
  values.map((value) => ({ value, label: label[value] }));
const HOME_QUALITY = opts<StreamQuality>(["original", "320", "256", "192", "128"], QUALITY_LABEL);
const AWAY_QUALITY = opts<StreamQuality>(["320", "original", "192", "128"], QUALITY_LABEL);
const SHUFFLE = [
  { value: "spread", label: "Spread" },
  { value: "standard", label: "Standard" },
] as const satisfies SelectOption<Settings["shuffleStyle"]>[];
const POPULAR = [
  { value: "global", label: "Global popularity" },
  { value: "myPlays", label: "My plays" },
] as const satisfies SelectOption<PopularSource>[];
const MOTION = [
  { value: "system", label: "Match system" },
  { value: "on", label: "On" },
  { value: "off", label: "Off" },
] as const satisfies SelectOption<ReduceMotion>[];
const LIKED_MEANS = [
  { value: "five", label: "5 stars" },
  { value: "fourUp", label: "4 stars or more" },
] as const satisfies SelectOption<Settings["likedMeans"]>[];
const LIKE_ROWS = [
  { value: "hover", label: "On hover" },
  { value: "always", label: "Always" },
  { value: "off", label: "Off" },
] as const satisfies SelectOption<Settings["likeControlRows"]>[];
const LYRICS_MODE = [
  { value: "autoShow", label: "Auto-show when available" },
  { value: "off", label: "Off" },
] as const satisfies SelectOption<Settings["lyricsMode"]>[];
const DISCORD_STATUS_LINE = [
  { value: "app", label: "Platter" },
  { value: "artist", label: "Artist" },
  { value: "song", label: "Song" },
] as const satisfies SelectOption<Settings["discordStatusLine"]>[];
const ART_SIZES_MB = [512, 1024, 2048, 5120, 10240, 20480];
const LEVELING = [
  { value: "smart", label: "Smart" },
  { value: "track", label: "Track" },
  { value: "album", label: "Album" },
  { value: "off", label: "Off" },
] as const satisfies SelectOption<Settings["leveling"]>[];

/** Where the app lives while its window is closed. */
const TRAY = platform === "mac" ? "menu bar" : "tray";

const CONNECTION_LABEL: Record<ConnectionKind, string> = {
  local: "Local connection",
  remote: "Remote access",
  relay: "Plex relay",
};

/** Main open-source parts Platter is built on (shown under About). */
/** The now-playing group lives outside the row list; show it when the search fits. */
function nowPlayingMatches(query: string): boolean {
  const q = query.trim().toLowerCase();
  return !q || ["now playing", "lyrics", "open", "song starts", "panel"].some((w) => w.includes(q) || q.includes(w));
}

export const NOTICES: { name: string; licence: string }[] = [
  { name: "Tauri", licence: "MIT or Apache-2.0" },
  { name: "React", licence: "MIT" },
  { name: "Zustand", licence: "MIT" },
  { name: "TanStack Virtual", licence: "MIT" },
  { name: "Lucide icons", licence: "ISC" },
  { name: "Inter typeface", licence: "SIL Open Font License 1.1" },
  { name: "Tokio", licence: "MIT" },
  { name: "Serde", licence: "MIT or Apache-2.0" },
  { name: "reqwest", licence: "MIT or Apache-2.0" },
  { name: "rustls", licence: "Apache-2.0, ISC or MIT" },
  { name: "Symphonia", licence: "MPL-2.0" },
  { name: "cpal", licence: "Apache-2.0" },
  { name: "rusqlite", licence: "MIT" },
  { name: "SQLite", licence: "Public domain" },
  { name: "keyring", licence: "MIT or Apache-2.0" },
  { name: "souvlaki", licence: "MIT" },
  { name: "mdns-sd", licence: "MIT or Apache-2.0" },
  { name: "tokio-rustls", licence: "MIT or Apache-2.0" },
  { name: "RustCrypto MD5", licence: "MIT or Apache-2.0" },
  { name: "RustCrypto SHA-2", licence: "MIT or Apache-2.0" },
  { name: "whisper.cpp and ggml (transcription)", licence: "MIT" },
  { name: "whisper-rs", licence: "Unlicense" },
  { name: "Whisper speech models (downloaded when asked)", licence: "MIT" },
];

/** Outside services (artist info, public share links), each under its own terms (shown under About). */
export const SOURCE_NOTICES: { name: string; licence: string }[] = [
  { name: "Wikipedia (artist bios)", licence: "CC BY-SA 4.0" },
  { name: "Wikidata (artist facts)", licence: "CC0" },
  { name: "Last.fm (with your key)", licence: "Last.fm API terms" },
  { name: "Fanart.tv (with your key)", licence: "Fanart.tv terms" },
  { name: "TheAudioDB (with your key)", licence: "TheAudioDB terms" },
  { name: "ListenBrainz (popular songs)", licence: "CC0" },
  { name: "MusicBrainz (ids for featured artists)", licence: "CC0" },
  { name: "iTunes Search (finding public links)", licence: "Apple iTunes Search API terms" },
  { name: "song.link (shared link pages)", licence: "Songlink terms" },
  { name: "Discord (status, when Discord is open)", licence: "Discord Developer Terms" },
  { name: "Cover Art Archive (covers on Discord)", licence: "Cover art owners' terms" },
  { name: "Imgur (covers on Discord, with your client ID)", licence: "Imgur API terms" },
  { name: "LRCLIB (lyrics, when turned on)", licence: "LRCLIB terms" },
  { name: "Hugging Face (transcription model download, when asked)", licence: "Hugging Face terms" },
];

// ------------------------------------------------------------------ rows

/** A Settings row note in the error colour (a refused API key). */
function errorNote(text: string | undefined): ReactNode {
  return text ? (
    <span className="info-key__error" role="alert">
      {text}
    </span>
  ) : undefined;
}

interface RowDef {
  key: string;
  section: string;
  title: string;
  description: string;
  /** Extra line under the description (state, notes). Not searched. */
  note?: ReactNode;
  /** A separate line below the description. */
  footnote?: string;
  /** A named group inside the section (shown as a small heading). */
  group?: string;
  control: (ids: { title: string; desc: string }) => ReactNode;
  /** A wide part under the row (the EQ editor). */
  block?: () => ReactNode;
}

function Row({ row }: { row: RowDef }) {
  const id = useId();
  const ids = { title: `${id}-t`, desc: `${id}-d` };
  return (
    <div className="setting-row" data-setting={row.key}>
      <div className="setting-row__text">
        <div id={ids.title} className="setting-row__title">
          {row.title}
        </div>
        <p id={ids.desc} className="setting-row__desc">
          {row.description}
          {row.note && (
            <>
              {" "}
              <span className="setting-row__note">{row.note}</span>
            </>
          )}
        </p>
        {row.footnote && <p className="setting-row__desc">{row.footnote}</p>}
      </div>
      <div className="setting-row__control">{row.control(ids)}</div>
      {row.block && <div className="setting-row__block">{row.block()}</div>}
    </div>
  );
}

const loading = <Skeleton width={120} height={32} radius="pill" />;

function toggle(s: Settings | null, key: keyof Settings, ids: { title: string; desc: string }, onChange?: (v: boolean) => void) {
  if (!s) return <Skeleton width={40} height={24} radius="pill" />;
  return (
    <Switch
      checked={!!s[key]}
      onChange={onChange ?? ((v) => updateSettings({ [key]: v }))}
      labelledBy={ids.title}
      describedBy={ids.desc}
    />
  );
}

function select<T extends string>(
  s: Settings | null,
  value: (s: Settings) => T,
  options: readonly SelectOption<T>[],
  ids: { title: string; desc: string },
  onChange: (v: T) => void,
) {
  if (!s) return loading;
  return <Select value={value(s)} options={options} onChange={onChange} labelledBy={ids.title} describedBy={ids.desc} />;
}

/** 1 to 50; applies on every valid entry, snaps back on blur if invalid. */
function PrecacheField({ value, ids }: { value: number; ids: { title: string; desc: string } }) {
  const [text, setText] = useState(String(value));
  useEffect(() => setText(String(value)), [value]);
  return (
    <input
      type="number"
      inputMode="numeric"
      min={1}
      max={50}
      step={1}
      className="number-field"
      value={text}
      aria-labelledby={ids.title}
      aria-describedby={ids.desc}
      onChange={(e) => {
        setText(e.target.value);
        const n = Number(e.target.value);
        if (Number.isInteger(n) && n >= 1 && n <= 50 && n !== value) void updateSettings({ precacheCount: n });
      }}
      onBlur={() => setText(String(value))}
    />
  );
}

// ------------------------------------------------------------------ dialogs

function ConfirmDialog({
  open,
  title,
  body,
  confirm,
  onConfirm,
  onClose,
}: {
  open: boolean;
  title: string;
  body: string;
  confirm: string;
  onConfirm: () => void;
  onClose: () => void;
}) {
  return (
    <Dialog open={open} onClose={onClose} label={title} className="confirm">
      <h2 className="confirm__title">{title}</h2>
      <p className="confirm__body">{body}</p>
      <div className="confirm__actions">
        <Button variant="ghost" onClick={onClose}>
          Cancel
        </Button>
        <Button
          variant="primary"
          onClick={() => {
            onClose();
            onConfirm();
          }}
        >
          {confirm}
        </Button>
      </div>
    </Dialog>
  );
}

function ServerDialog({ open, onClose, current }: { open: boolean; onClose: () => void; current?: string | null }) {
  const [servers, setServers] = useState<ServerInfo[] | null>(null);
  const [err, setErr] = useState<string | null>(null);
  useEffect(() => {
    if (!open) return;
    setServers(null);
    setErr(null);
    backend.serversList().then(setServers, (e: AppError) => setErr(e.message));
  }, [open]);
  const pick = (s: ServerInfo) =>
    backend.serverSelect(s.machineId).then(
      () => {
        onClose();
        if (s.machineId !== current) toast.info(`Switched to ${s.name}`);
      },
      (e: AppError) => setErr(e.message),
    );
  return (
    <Dialog open={open} onClose={onClose} label="Switch server" className="confirm">
      <h2 className="confirm__title">Switch server</h2>
      <ul className="server-list">
        {servers
          ? servers.map((s) => (
              <li key={s.machineId}>
                <button
                  type="button"
                  className="server-item"
                  disabled={!s.online}
                  aria-current={s.machineId === current ? "true" : undefined}
                  onClick={() => void pick(s)}
                >
                  <Server size={20} aria-hidden="true" />
                  <span className="server-item__name">{s.name}</span>
                  <span className={`server-item__state ${s.online ? "is-online" : ""}`}>
                    {s.machineId === current ? "Current" : s.online ? "Online" : "Offline"}
                  </span>
                </button>
              </li>
            ))
          : [0, 1].map((i) => (
              <li key={i}>
                <Skeleton height={56} radius="card" />
              </li>
            ))}
      </ul>
      {err && (
        <p className="confirm__error" role="alert">
          {err}
        </p>
      )}
      <div className="confirm__actions">
        <Button variant="ghost" onClick={onClose}>
          Cancel
        </Button>
      </div>
    </Dialog>
  );
}

function NoticesDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  return (
    <Dialog open={open} onClose={onClose} label="Open-source notices" className="confirm">
      <h2 className="confirm__title">Open-source notices</h2>
      <p className="confirm__body">Platter is built on these open-source projects. Thanks to everyone who works on them.</p>
      <ul className="notices">
        {NOTICES.map((n) => (
          <li key={n.name} className="notices__item">
            <span className="notices__name">{n.name}</span>
            <span className="notices__licence">{n.licence}</span>
          </li>
        ))}
      </ul>
      <p className="confirm__body notices__sub">Artist info and public share links come from these services when they are on.</p>
      <ul className="notices">
        {SOURCE_NOTICES.map((n) => (
          <li key={n.name} className="notices__item">
            <span className="notices__name">{n.name}</span>
            <span className="notices__licence">{n.licence}</span>
          </li>
        ))}
      </ul>
      <div className="confirm__actions">
        <Button variant="primary" onClick={onClose}>
          Close
        </Button>
      </div>
    </Dialog>
  );
}

// ------------------------------------------------------------------ page

type UpdateState = { kind: "idle" } | { kind: "checking" } | { kind: "done"; info: UpdateInfo } | { kind: "error"; message: string };

function matches(row: RowDef, q: string): boolean {
  if (!q) return true;
  const hay = `${row.section} ${row.group ?? ""} ${row.title} ${row.description}`.toLowerCase();
  return q
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean)
    .every((w) => hay.includes(w));
}

/** Settings (opened from the header or with Ctrl/Cmd+,). */
export function SettingsPage() {
  const s = useSettings((x) => x.settings);
  const [sidebarArt, setSidebarArt] = useSidebarArt();
  const auth = useApp((x) => x.auth);
  const healthConn = useApp((x) => x.health?.connection);
  const syncing = useApp((x) => !!x.sync && x.sync.phase !== "idle");
  const [query, setQuery] = useState("");
  const [art, setArt] = useState<ArtCacheInfo | null>(null);
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [update, setUpdate] = useState<UpdateState>({ kind: "idle" });
  const [dialog, setDialog] = useState<"signOut" | "server" | "notices" | null>(null);

  const { keys: infoKeys, setKey: setInfoKey } = useInfoKeys();
  const { status: discord, setKey: setImgurKey } = useDiscordStatus();
  const discordOff = !!discord && !discord.available;
  const artMb = s?.artCacheMb;
  useEffect(() => {
    let alive = true;
    backend.artCacheInfo().then((a) => alive && setArt(a), () => {});
    return () => {
      alive = false;
    };
  }, [artMb]);
  useEffect(() => {
    backend.appInfo().then(setInfo, () => {});
  }, []);

  const conn = healthConn ?? auth?.server?.connection ?? null;
  const away = conn === "remote" || conn === "relay";

  const clearArt = () =>
    backend.artCacheClear().then(
      (freed) => {
        toast.info(freed > 0 ? `Cleared ${formatBytes(freed)} of artwork` : "The artwork cache was already empty");
        return backend.artCacheInfo().then(setArt);
      },
      (e) => toast.error(`Couldn't clear the cache${errText(e) ? `: ${errText(e)}` : ""}`),
    );
  const resync = () =>
    backend.syncNow(true).catch((e) => toast.error(`Couldn't start a resync${errText(e) ? `: ${errText(e)}` : ""}`));
  const checkUpdates = () => {
    setUpdate({ kind: "checking" });
    backend.updateCheck().then(
      (i) => setUpdate({ kind: "done", info: i }),
      (e) => setUpdate({ kind: "error", message: errText(e) || "Couldn't check for updates" }),
    );
  };
  const clearArtistInfo = () =>
    backend.artistInfoCacheClear().then(
      () => toast.info("Artist info cleared"),
      (e) => toast.error(`Couldn't clear artist info${errText(e) ? `: ${errText(e)}` : ""}`),
    );
  const clearRecentSearches = () =>
    backend.searchRecentsClear().then(
      () => toast.info("Recent searches cleared"),
      (e) => toast.error(`Couldn't clear recent searches${errText(e) ? `: ${errText(e)}` : ""}`),
    );
  const signOut = () => backend.authSignOut().catch((e) => toast.error(`Couldn't sign out${errText(e) ? `: ${errText(e)}` : ""}`));

  const artOptions: SelectOption<string>[] = [...new Set([...ART_SIZES_MB, ...(artMb ? [artMb] : [])])]
    .sort((a, b) => a - b)
    .map((mb) => ({ value: String(mb), label: formatBytes(mb * 1024 * 1024) }));

  const version = info?.version;
  const updateNote =
    update.kind === "checking"
      ? "Checking…"
      : update.kind === "error"
        ? update.message
        : update.kind === "done"
          ? update.info.available
            ? `Version ${update.info.version ?? "new"} is ready. Restart to install it.`
            : "You're up to date."
          : undefined;

  const rows: RowDef[] = [
    // Account
    {
      key: "account",
      section: "Account",
      title: auth?.username ? `Signed in as ${auth.username}` : "Signed in",
      description: auth?.server
        ? `${auth.server.name}${conn ? `, ${CONNECTION_LABEL[conn]}` : ""}`
        : "No server selected",
      control: () => (
        <>
          <Button size="sm" onClick={() => setDialog("server")}>
            Switch server
          </Button>
          <Button size="sm" onClick={() => setDialog("signOut")}>
            Sign out
          </Button>
        </>
      ),
    },
    {
      key: "showPlexAvatar",
      section: "Account",
      title: "Show my Plex profile picture",
      description: "Shows your Plex account's picture on the app menu button. Off shows the first letter of your name. This device only.",
      control: (ids) => toggle(s, "showPlexAvatar", ids),
    },
    // Audio quality
    {
      key: "qualityHome",
      section: "Audio quality",
      title: "Streaming quality at home",
      description: "Used on your home network. Original plays the file as it is. Changes apply from the next track.",
      note: away ? "You're away from home right now." : undefined,
      control: (ids) => select(s, (x) => x.qualityHome, HOME_QUALITY, ids, (v) => updateSettings({ qualityHome: v })),
    },
    {
      key: "qualityAway",
      section: "Audio quality",
      title: "Streaming quality away from home",
      description: "Used over remote access or the Plex relay. The relay stays capped by the server.",
      note:
        conn === "relay"
          ? "On the relay now, so Plex limits streams to 128 kbps."
          : !away && conn
            ? "You're at home right now."
            : undefined,
      control: (ids) => select(s, (x) => x.qualityAway, AWAY_QUALITY, ids, (v) => updateSettings({ qualityAway: v })),
    },
    {
      key: "showFormatBadge",
      section: "Audio quality",
      title: "Show format badge",
      description: "Shows the format, bit depth and sample rate (like FLAC 24/96) next to songs and albums.",
      control: (ids) => toggle(s, "showFormatBadge", ids),
    },
    // Playback
    {
      key: "shuffleStyle",
      section: "Playback",
      title: "Shuffle style",
      description: "Spread keeps songs by the same artist apart. Standard is a plain random order.",
      control: (ids) => select(s, (x) => x.shuffleStyle, SHUFFLE, ids, (v) => updateSettings({ shuffleStyle: v })),
    },
    {
      key: "resumeContexts",
      section: "Playback",
      title: "Albums and playlists resume where you left off",
      description: "Playing an album or playlist again picks up at the song and spot you stopped. Off starts at the first song.",
      control: (ids) => toggle(s, "resumeContexts", ids),
    },
    {
      key: "precacheCount",
      section: "Playback",
      title: "Pre-cache ahead",
      description: "How many upcoming songs to save on this computer so they start instantly and survive a dropped connection. 1 to 50.",
      control: (ids) => (s ? <PrecacheField value={s.precacheCount} ids={ids} /> : loading),
    },
    {
      key: "reportToPlex",
      section: "Playback",
      title: "Report plays to Plex",
      description: "Lets Plex count plays and show what's playing in your other Plex apps.",
      control: (ids) => toggle(s, "reportToPlex", ids),
    },
    {
      key: "privateSession",
      section: "Playback",
      title: "Private session",
      description: "Plays aren't saved to your listening history on this device. Plex still counts them unless you turn that off above.",
      control: (ids) => toggle(s, "privateSession", ids),
    },
    {
      key: "outputDevice",
      section: "Playback",
      title: "Output device",
      description: "Where music plays on this computer. System default follows your computer's sound setting. If the device you pick is unplugged, Platter pauses and switches to the system default.",
      control: (ids) => <OutputDeviceSelect ids={ids} />,
    },
    {
      key: "abLoopHandles",
      section: "Playback",
      title: "A-B loop on the seek bar",
      description: "Right-click or Alt+click the seek bar to mark a start, then an end. The part between them repeats until you clear it with Esc or the A-B chip.",
      control: (ids) => toggle(s, "abLoopHandles", ids),
    },
    // Library
    {
      key: "popularSource",
      section: "Library",
      title: "Artist page Popular",
      description: "Rank an artist's popular songs by how many people play them (from ListenBrainz, or Last.fm top tracks), or by your own plays. With neither source on, your plays are used.",
      control: (ids) => select(s, (x) => x.popularSource, POPULAR, ids, (v) => updateSettings({ popularSource: v })),
    },
    {
      key: "linkFeatured",
      section: "Library",
      title: "Link featured artists",
      description: "Every name in a song's artist line opens a page, including featured artists who have no albums in your library. Their pages show the songs that credit them.",
      control: (ids) => toggle(s, "linkFeatured", ids),
    },
    {
      key: "artCache",
      section: "Library",
      title: "Artwork and cache size",
      description: "Cover art is kept on this computer up to this size. Clearing it frees the space; art downloads again as you browse.",
      note: art ? `Using ${formatBytes(art.usedBytes)}.` : undefined,
      control: (ids) => (
        <>
          {s ? (
            <Select
              value={String(s.artCacheMb)}
              options={artOptions}
              onChange={(v) => updateSettings({ artCacheMb: Number(v) })}
              labelledBy={ids.title}
              describedBy={ids.desc}
            />
          ) : (
            loading
          )}
          <Button size="sm" onClick={() => void clearArt()}>
            Clear cache
          </Button>
        </>
      ),
    },
    {
      key: "resync",
      section: "Library",
      title: "Resync library",
      description: "Reads your whole library from Plex again. Use it if something looks out of date. Progress shows at the top.",
      note: syncing ? "Syncing now." : undefined,
      control: () => (
        <Button size="sm" disabled={syncing} onClick={() => void resync()}>
          Resync library
        </Button>
      ),
    },
    // Search
    {
      key: "searchSuggestions",
      section: "Search",
      title: "Show suggestions as you type",
      description: "Completions and matching artists, albums, songs and playlists appear under the search box while you type.",
      control: (ids) => toggle(s, "searchSuggestions", ids),
    },
    {
      key: "searchRecents",
      section: "Search",
      title: "Remember recent searches",
      description: "Things you open from search show up when you click the empty search box. Kept on this computer only.",
      control: (ids) => (
        <>
          {toggle(s, "searchRecents", ids)}
          <Button size="sm" onClick={() => void clearRecentSearches()}>
            Clear recent searches
          </Button>
        </>
      ),
    },
    // Appearance
    {
      key: "sidebarArt",
      section: "Appearance",
      title: "Show the library sidebar's art",
      description: "Shows cover art next to each item in the library's list views. This device only.",
      control: (ids) =>
        s ? (
          <Switch checked={sidebarArt} onChange={setSidebarArt} labelledBy={ids.title} describedBy={ids.desc} />
        ) : (
          <Skeleton width={40} height={24} radius="pill" />
        ),
    },
    {
      key: "artworkColours",
      section: "Appearance",
      title: "Artwork colours",
      description: "Album, artist and playlist headers take their colour from the artwork. Off uses one neutral colour.",
      control: (ids) => toggle(s, "artworkColours", ids),
    },
    {
      key: "reduceMotion",
      section: "Appearance",
      title: "Reduce motion",
      description: "Turns off movement like sliding and scaling. Fades stay. Match system follows your computer's setting.",
      control: (ids) => select(s, (x) => x.reduceMotion, MOTION, ids, (v) => updateSettings({ reduceMotion: v })),
    },
    {
      key: "upNextLine",
      section: "Appearance",
      title: "Show what's next in the player bar",
      description: "In the last seconds of a song, the line under its title shows the next song. Click it to open the queue.",
      control: (ids) => toggle(s, "upNextLine", ids),
    },
    {
      key: "volumePercent",
      section: "Appearance",
      title: "Show volume percentage",
      description: "Shows the volume as a percentage above the slider for a moment while you change it.",
      control: (ids) => toggle(s, "volumePercent", ids),
    },
    // Home
    {
      key: "homeShortcuts",
      section: "Home",
      title: "Show the shortcut grid",
      description: "Shows up to 8 things you've played lately or play most at the top of Home.",
      control: (ids) => toggle(s, "homeShortcuts", ids),
    },
    {
      key: "homeColourWash",
      section: "Home",
      title: "Colour wash from artwork",
      description: "Tints the top of Home with the colour of the shortcut you point at.",
      note: s && !s.artworkColours ? "Turn on Artwork colours to use this." : undefined,
      control: (ids) =>
        s ? (
          <Switch
            checked={s.artworkColours && s.homeColourWash}
            disabled={!s.artworkColours}
            onChange={(v) => updateSettings({ homeColourWash: v })}
            labelledBy={ids.title}
            describedBy={ids.desc}
          />
        ) : (
          <Skeleton width={40} height={24} radius="pill" />
        ),
    },
    // App
    {
      key: "updates",
      section: "App",
      title: "Check for updates",
      description: version ? `You have version ${version}.` : "Platter checks for new versions on its own every few hours.",
      note: updateNote,
      control: () =>
        update.kind === "done" && update.info.available ? (
          <Button size="sm" variant="primary" onClick={() => void backend.updateInstallOnRestart()}>
            Restart to install
          </Button>
        ) : (
          <Button size="sm" disabled={update.kind === "checking"} onClick={checkUpdates}>
            Check for updates
          </Button>
        ),
    },
    {
      key: "mediaSession",
      section: "App",
      title: "Show in the system media controls",
      description:
        platform === "mac"
          ? "Shows the song, artist and cover in Control Center and on the lock screen, with play, pause, next and previous."
          : "Shows the song, artist and cover in your computer's media controls and on the lock screen, with play, pause, next and previous.",
      control: (ids) => toggle(s, "mediaSession", ids),
    },
    {
      key: "mediaKeys",
      section: "App",
      title: "Media keys",
      description: "The play, next and previous keys on your keyboard or headset control Platter, even when it's in the background.",
      note: s?.mediaSession ? "While Platter is in the system media controls, your computer sends these keys there." : undefined,
      control: (ids) => toggle(s, "mediaKeys", ids),
    },
    {
      key: "closeToTray",
      section: "App",
      title: `Close to the ${TRAY}`,
      description: `Closing the window keeps Platter playing in the ${TRAY}. Click its icon to bring the window back, or quit from its menu.`,
      control: (ids) => toggle(s, "closeToTray", ids),
    },
    {
      key: "launchAtLogin",
      section: "App",
      title: "Open Platter when you sign in",
      description: `Starts Platter when you sign in to your computer. It starts hidden in the ${TRAY} if Close to the ${TRAY} is on, otherwise minimised.`,
      control: (ids) => toggle(s, "launchAtLogin", ids),
    },
    {
      key: "notifyTrackChange",
      section: "App",
      title: "Notify on track change",
      description: "Shows a desktop notification when a new song starts, unless Platter is in front, a private session is on, or you're skipping through songs. This device only.",
      control: (ids) => toggle(s, "notifyTrackChange", ids),
    },
    // About
    {
      key: "about",
      section: "About",
      title: `Platter${version ? ` ${version}` : ""}`,
      description: `A music player for Plex. Free software under the ${info?.licence ?? "GPL-3.0-or-later"} licence.`,
      footnote: "Not affiliated with or endorsed by Plex, Inc.",
      control: () => (
        <>
          <Button size="sm" onClick={() => setDialog("notices")}>
            Open-source notices
          </Button>
          <Button size="sm" onClick={() => void backend.openProjectPage().catch(() => toast.error("Couldn't open the browser"))}>
            Project page
          </Button>
        </>
      ),
    },
    // Library: playlists
    {
      key: "playlistAskDuplicates",
      section: "Library",
      title: "Ask before adding a song that's already in a playlist",
      description: "When a song you add from a menu or by dragging is already in that playlist, ask whether to add it again.",
      control: (ids) => toggle(s, "playlistAskDuplicates", ids),
    },
    {
      key: "playlistNewName",
      section: "Library",
      title: "New playlists start with name",
      description: "The name a new playlist gets. #N becomes the next free number, so you get My playlist #1, #2 and so on.",
      control: (ids) => <PlaylistNameSetting ids={ids} />,
    },
    // Library: likes
    {
      key: "likedMeans",
      section: "Library",
      title: "Liked means",
      description: "Likes are Plex star ratings, so other Plex apps see them. Liking a song always gives it five stars; this picks which ratings count as liked.",
      control: (ids) => select(s, (x) => x.likedMeans, LIKED_MEANS, ids, (v) => void updateSettings({ likedMeans: v }).then(reloadLikes)),
    },
    {
      key: "likeControlRows",
      section: "Library",
      title: "Show the like control on song rows",
      description: "On hover shows the plus when you point at a song (liked songs always show their tick). Always shows it on every row; Off hides it in rows.",
      control: (ids) => select(s, (x) => x.likeControlRows, LIKE_ROWS, ids, (v) => updateSettings({ likeControlRows: v })),
    },
    // Integrations: artist info
    {
      key: "infoWikipedia",
      section: "Integrations",
      group: "Artist info",
      title: "Wikipedia and Wikidata",
      description: "Longer artist bios, where an artist is from, years active and band members. Only the artist's name or MusicBrainz id is sent.",
      control: (ids) => toggle(s, "infoWikipedia", ids),
    },
    {
      key: "infoLastfm",
      section: "Integrations",
      group: "Artist info",
      title: "Last.fm",
      description: "Listener counts, tags and a bio when Wikipedia has none. Needs your own free Last.fm API key, kept in this computer's keychain.",
      note: errorNote(keyNote("lastfm", infoKeys)),
      control: (ids) => <InfoKeyControl source="lastfm" ids={ids} keys={infoKeys} setKey={setInfoKey} />,
    },
    {
      key: "infoFanart",
      section: "Integrations",
      group: "Artist info",
      title: "Fanart.tv",
      description: "Banners, logos and photos for artists Plex has no art for. Needs your Fanart.tv API key, kept in this computer's keychain.",
      note: errorNote(keyNote("fanart", infoKeys)),
      control: (ids) => <InfoKeyControl source="fanart" ids={ids} keys={infoKeys} setKey={setInfoKey} />,
    },
    {
      key: "infoAudiodb",
      section: "Integrations",
      group: "Artist info",
      title: "TheAudioDB",
      description: "A photo and bio when nothing else has one. Needs your own TheAudioDB key; its shared test key isn't for use in apps.",
      note: errorNote(keyNote("audiodb", infoKeys)),
      control: (ids) => <InfoKeyControl source="audiodb" ids={ids} keys={infoKeys} setKey={setInfoKey} />,
    },
    {
      key: "infoListenbrainz",
      section: "Integrations",
      group: "Artist info",
      title: "ListenBrainz (popular songs)",
      description: "Ranks an artist's Popular songs by how many people listen to them. Only the artist's MusicBrainz id is sent, and for featured artists their name goes to MusicBrainz to find it.",
      control: (ids) => toggle(s, "infoListenbrainz", ids),
    },
    {
      key: "infoListenbrainzToken",
      section: "Integrations",
      group: "Artist info",
      title: "ListenBrainz token",
      description: "Optional. ListenBrainz answers some artists only with a user token. Copy yours from your ListenBrainz settings page; it's kept in this computer's keychain.",
      note: errorNote(keyNote("listenbrainz", infoKeys)),
      control: (ids) => <InfoKeyControl source="listenbrainz" ids={ids} keys={infoKeys} setKey={setInfoKey} />,
    },
    {
      key: "infoCache",
      section: "Integrations",
      group: "Artist info",
      title: "Clear artist info cache",
      description: "Artist info is kept on this computer for 30 days. Clearing it fetches everything again as you open artists.",
      control: () => (
        <Button size="sm" onClick={() => void clearArtistInfo()}>
          Clear artist info
        </Button>
      ),
    },
    // Playback: listen on
    {
      key: "showPlexPlayers",
      section: "Playback",
      title: "Show Plex players in the device picker",
      description: "Lists Plexamp, Plex apps and Sonos speakers linked to Plex under Listen on, so you can move what's playing to them.",
      control: (ids) => toggle(s, "showPlexPlayers", ids),
    },
    {
      key: "moveStopsOldDevice",
      section: "Playback",
      title: "Stop on the old device when moving playback",
      description: "Moving playback to another device pauses it here, and bringing it back stops it there. Off leaves both playing.",
      control: (ids) => toggle(s, "moveStopsOldDevice", ids),
    },
    {
      key: "actAsPlexPlayer",
      section: "Playback",
      title: "Let other Plex apps play on this computer",
      description: "Platter shows up as a player in Plexamp and other Plex apps on your network, so they can send music here. Opens a network port, so your computer may ask to allow it. This device only.",
      control: (ids) => toggle(s, "actAsPlexPlayer", ids),
    },
    {
      key: "showCastDevices",
      section: "Playback",
      title: "Show Chromecast and AirPlay devices",
      description: "Lists Chromecasts, Google speakers and groups, and AirPlay speakers on your network under Listen on. Off stops Platter looking for them.",
      control: (ids) => toggle(s, "showCastDevices", ids),
    },
    // Transitions
    {
      key: "crossfade",
      section: "Transitions",
      title: "Crossfade",
      description:
        "Smart overlaps only the quiet end of a song and the quiet start of the next, so songs that end cold aren't cut and fades keep their shape. Fixed fades over the time you set. Skipping always cuts straight to the next song.",
      control: (ids) => <CrossfadeControl ids={ids} />,
    },
    {
      key: "crossfadeSkipAlbums",
      section: "Transitions",
      title: "Never crossfade within an album",
      description: "Songs from the same album played in album order run straight into each other, with no gap and no fade.",
      control: (ids) => toggle(s, "crossfadeSkipAlbums", ids),
    },
    // Loudness
    {
      key: "leveling",
      section: "Loudness",
      title: "Loudness leveling",
      description:
        "Plays songs at a similar volume using Plex's loudness analysis. Smart keeps an album's own dynamics while you play it in order and levels each song otherwise.",
      control: (ids) => select(s, (x) => x.leveling, LEVELING, ids, (v) => updateSettings({ leveling: v })),
    },
    {
      key: "preampDb",
      section: "Loudness",
      title: "Preamp",
      description: "Raises or lowers every song, from -12 to +12 dB. Songs Plex hasn't analysed get only this. This device only.",
      control: (ids) => <PreampControl ids={ids} />,
    },
    {
      key: "limiter",
      section: "Loudness",
      title: "Limiter",
      description: "Gently holds back peaks so the preamp and EQ boosts never distort. This device only.",
      control: (ids) => toggle(s, "limiter", ids),
    },
    // Equalizer
    {
      key: "eqEnabled",
      section: "Equalizer",
      title: "Equalizer",
      description:
        "Shape the sound with ten bands from 31 Hz to 16 kHz. Drag the points, or focus one and use the arrow keys (Shift for bigger steps). Pick a preset, save your own, and give an output device its own profile. This device only.",
      control: (ids) => toggle(s, "eqEnabled", ids),
      block: () => <EqEditor />,
    },
    // Sharing
    {
      key: "sharePublicLinks",
      section: "Sharing",
      title: "Public links for released music",
      description:
        "Share menus can copy a song.link page that opens the song or album in any music service. Platter finds it with iTunes Search, sending only the artist, title and album. Unreleased music never gets one. Off leaves only Platter links.",
      control: (ids) => toggle(s, "sharePublicLinks", ids),
    },
    // Integrations: Discord
    {
      key: "discordStatus",
      section: "Integrations",
      group: "Discord",
      title: "Show what you're playing on Discord",
      description: "Your Discord profile shows the song, artist, album and cover while music plays. Needs the Discord app open on this computer. A private session hides it. This device only.",
      note: discordNote(discord, s?.discordStatus),
      control: (ids) => (discordOff ? null : toggle(s, "discordStatus", ids)),
    },
    ...(discordOff
      ? []
      : ([
          {
            key: "discordClearOnPause",
            section: "Integrations",
            group: "Discord",
            title: "Clear status when paused",
            description: "Pausing removes the status. Off keeps the song on your profile, marked as paused.",
            control: (ids) => toggle(s, "discordClearOnPause", ids),
          },
          {
            key: "discordShowUnreleased",
            section: "Integrations",
            group: "Discord",
            title: "Show unreleased songs",
            description: "Off shows nothing on Discord while an unreleased song plays.",
            control: (ids) => toggle(s, "discordShowUnreleased", ids),
          },
          {
            key: "discordUnreleasedArt",
            section: "Integrations",
            group: "Discord",
            title: "Show unreleased cover art",
            description: "Off shows the Platter icon instead of the cover for unreleased songs, so their art is never looked up or uploaded.",
            control: (ids) => toggle(s, "discordUnreleasedArt", ids),
          },
          {
            key: "discordFindSong",
            section: "Integrations",
            group: "Discord",
            title: "Find this song button",
            description: "Adds a button that opens the song's song.link page, for released songs only.",
            note: s && !s.sharePublicLinks ? "Turn on Public links for released music to use this." : undefined,
            control: (ids) => toggle(s, "discordFindSong", ids),
          },
          {
            key: "discordCoverArchive",
            section: "Integrations",
            group: "Discord",
            title: "Covers from Cover Art Archive",
            description:
              "Finds a public copy of the album's cover by its MusicBrainz id. For released albums without one, MusicBrainz is searched by album artist and title, and only an exact match is used.",
            control: (ids) => toggle(s, "discordCoverArchive", ids),
          },
          {
            key: "discordUploadArt",
            section: "Integrations",
            group: "Discord",
            title: "Upload other covers to Imgur",
            description: "Covers Cover Art Archive doesn't have are uploaded once to Imgur so Discord can show them. Anyone with the link can see an uploaded cover. Needs your own Imgur client ID, kept in this computer's keychain.",
            note: errorNote(imgurNote(discord)),
            control: (ids) => <ImgurKeyControl ids={ids} status={discord} setKey={setImgurKey} />,
          },
          {
            key: "discordUploadCatbox",
            section: "Integrations",
            group: "Discord",
            title: "Upload covers to catbox.moe",
            description:
              "Covers Cover Art Archive doesn't have are uploaded once to catbox.moe, which needs no account. An uploaded cover is public to anyone with the link and can't be deleted. Imgur is used instead when it's set up.",
            note: errorNote(catboxNote(discord, s?.discordUploadCatbox)),
            footnote:
              "Where covers come from: Cover Art Archive first, then an upload to Imgur or catbox.moe if one is on. Without a cover, Discord shows the Platter icon.",
            control: (ids) => toggle(s, "discordUploadCatbox", ids),
          },
          {
            key: "discordStatusLine",
            section: "Integrations",
            group: "Discord",
            title: "Status line shows",
            description: "What the member list shows after \"Listening to\": Platter, the artist, or the song.",
            control: (ids) =>
              select(s, (x) => x.discordStatusLine, DISCORD_STATUS_LINE, ids, (v) => updateSettings({ discordStatusLine: v })),
          },
        ] satisfies RowDef[])),
    // Lyrics and the full-window view
    {
      key: "lyricsMode",
      section: "Appearance",
      title: "Lyrics",
      description: "Auto-show opens the lyrics view when a song with synced lyrics starts while now playing is open. Off opens it only from the lyrics button.",
      control: (ids) => select(s, (x) => x.lyricsMode, LYRICS_MODE, ids, (v) => updateSettings({ lyricsMode: v })),
    },
    {
      key: "lyricsLrclib",
      section: "Appearance",
      title: "Lyrics from LRCLIB when Plex has none",
      description:
        "Looks the song up on LRCLIB, a free lyrics library, sending its artist, title, album and length. Songs it doesn't know are searched by artist and title only. Results are kept for 30 days.",
      control: (ids) => toggle(s, "lyricsLrclib", ids),
    },
    {
      key: "lyricsPreferOwn",
      section: "Appearance",
      title: "Prefer my own lyrics over Plex's",
      description:
        "Songs you've written lyrics for in Platter show yours, even when Plex has some. Off shows yours only when Plex has none, unless you chose otherwise for the song.",
      control: (ids) => toggle(s, "lyricsPreferOwn", ids),
    },
    {
      key: "lyricsShowAdd",
      section: "Appearance",
      title: "Show Add lyrics on songs without lyrics",
      description: "The lyrics view offers to write lyrics for a song that has none. Add lyrics stays in the song's menu either way.",
      control: (ids) => toggle(s, "lyricsShowAdd", ids),
    },
    {
      key: "fullWindowHideControls",
      section: "Appearance",
      title: "Full-window view hides controls",
      description: "After 3 seconds without moving the mouse, the full-window view fades its controls. Moving the mouse or pressing a key brings them back.",
      control: (ids) => toggle(s, "fullWindowHideControls", ids),
    },
    // Library: tags read from the files
    {
      key: "readFileTags",
      section: "Library",
      group: "File tags",
      title: "Read extra tags from files",
      description:
        "Reads explicit, version and edition tags from your music files through Plex, a few kilobytes at a time and only while nothing is downloading for playback. Your files are never changed. This device only.",
      note: <TagScanNote />,
      control: (ids) => toggle(s, "readFileTags", ids),
    },
    {
      key: "explicitContent",
      section: "Library",
      group: "File tags",
      title: "Explicit content",
      description:
        "Filtered leaves songs tagged explicit out of albums, playlists and anything else you play, and says how many were skipped. Applies the next time something starts.",
      control: (ids) => select(s, (x) => x.explicitContent, EXPLICIT_OPTIONS, ids, (v) => updateSettings({ explicitContent: v })),
    },
    {
      key: "showFanEdits",
      section: "Library",
      group: "File tags",
      title: "Show fan and AI edits",
      description: "Songs tagged as fan edits or AI edits show in albums and play. Off hides them, and albums made only of them.",
      control: (ids) => toggle(s, "showFanEdits", ids),
    },
    {
      key: "groupEditions",
      section: "Library",
      group: "File tags",
      title: "Group album editions",
      description:
        "Albums that share a group tag, like a deluxe edition or builds of an unreleased album, show as one card. The album page gets an edition picker.",
      control: (ids) => toggle(s, "groupEditions", ids),
    },
    // Library: drag and drop
    {
      key: "dragAndDrop",
      section: "Library",
      group: "Drag and drop",
      title: "Drag and drop to add to playlists and the queue",
      description: "Drag songs, albums and playlists onto a playlist or Liked Songs in Your Library, or into the queue. Reordering a playlist or the queue by dragging works either way.",
      control: (ids) => toggle(s, "dragAndDrop", ids),
    },
    // Lyrics transcribed on this computer
    {
      key: "transcribeLyrics",
      section: "Appearance",
      group: "Lyrics transcription",
      title: "Transcribe songs without lyrics on this computer",
      description:
        "Adds Transcribe this song to the lyrics view and the album menu. A speech model is downloaded once, and songs are transcribed on this computer only; the audio never leaves it. Results may be wrong and can be fixed in the lyrics editor. This device only.",
      note: <TranscribeNote />,
      control: (ids) => toggle(s, "transcribeLyrics", ids),
    },
    {
      key: "transcribeModel",
      section: "Appearance",
      group: "Lyrics transcription",
      title: "Transcription model",
      description: "Small is faster. Medium is more accurate and takes about three times as long. The size is the download. This device only.",
      control: (ids) => <ModelControl s={s} ids={ids} />,
    },
    {
      key: "transcribeUnreleased",
      section: "Appearance",
      group: "Lyrics transcription",
      title: "Transcribe all unreleased songs without lyrics",
      description:
        "Works through them in the background with fewer processor cores. It pauses on battery power and while music plays over a slow connection, and always gives way to playback.",
      note: <BatchNote />,
      control: () => <BatchControl />,
    },
    {
      key: "showAutoLyrics",
      section: "Appearance",
      group: "Lyrics transcription",
      title: "Show auto-transcribed lyrics",
      description: "Off hides transcribed lyrics you haven't edited. They stay saved.",
      control: (ids) => toggle(s, "showAutoLyrics", ids),
    },
  ];

  const shown = rows.filter((r) => matches(r, query.trim()));
  // About stays last; later sections (Integrations) go in before it.
  const sections = [...new Set(shown.map((r) => r.section))].sort((a, b) => Number(a === "About") - Number(b === "About"));

  return (
    <div className="page-pad settings">
      <h1 className="page-title">Settings</h1>
      <div className="settings__search">
        <SearchField value={query} onChange={setQuery} label="Search settings" placeholder="Search settings" />
      </div>
      {sections.map((section) => {
        const id = `settings-${section.toLowerCase().replace(/\s+/g, "-")}`;
        return (
          <section key={section} className="settings__group" aria-labelledby={id}>
            <h2 id={id} className="settings__heading">
              {section}
            </h2>
            {shown
              .filter((r) => r.section === section)
              .map((r, i, list) => (
                <Fragment key={r.key}>
                  {r.group && r.group !== list[i - 1]?.group && <h3 className="settings__subheading">{r.group}</h3>}
                  <Row row={r} />
                </Fragment>
              ))}
          </section>
        );
      })}
      {shown.length === 0 && !nowPlayingMatches(query) && (
        <div className="settings__empty" role="status">
          <p className="settings__empty-title">No settings match "{query.trim()}"</p>
          <p className="settings__empty-hint">Try another word, like quality, cache or motion.</p>
        </div>
      )}
      {nowPlayingMatches(query) && <NowPlayingSettings />}
      <p className="settings__hint">Shortcut: {modKey}+,</p>
      <ConfirmDialog
        open={dialog === "signOut"}
        title="Sign out of Plex?"
        body="Platter forgets your Plex sign-in on this computer. You'll need to sign in again to play your music."
        confirm="Sign out"
        onConfirm={() => void signOut()}
        onClose={() => setDialog(null)}
      />
      <ServerDialog open={dialog === "server"} onClose={() => setDialog(null)} current={auth?.server?.machineId} />
      <NoticesDialog open={dialog === "notices"} onClose={() => setDialog(null)} />
    </div>
  );
}
