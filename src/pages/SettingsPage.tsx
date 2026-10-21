import { useLibraryChips } from "../lib/libraryChips";
import { ChevronLeft, ChevronRight, MoreHorizontal, RotateCcw, Server } from "lucide-react";
import { Fragment, memo, useEffect, useId, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { Button } from "../components/Button";
import { Dialog } from "../components/Dialog";
import { SearchField } from "../components/SearchField";
import { Select, type SelectOption } from "../components/Select";
import { usePhoneForm } from "../phone/form";
import { phoneRows } from "../phone/phoneSettings";
import { visualizerRows } from "../visualizer/settingsRows";
import { Skeleton } from "../components/Skeleton";
import { Switch } from "../components/Switch";
import { useSidebarArt, useUi } from "../stores/ui";
import { shortcutLabel } from "../shell/shortcuts";
import { backend } from "../lib/backend";
import { useDomId } from "../lib/domId";
import { SECTION_LABELS, formatBytes } from "../lib/format";
import { modKey, platform } from "../lib/platform";
import type {
  AppError,
  DiscordStatus,
  AppInfo,
  ArtCacheInfo,
  ConnectionKind,
  DownloadQuality,
  PopularSource,
  ReduceMotion,
  RecordMotion,
  CoverLight,
  ServerInfo,
  Settings,
  StreamQuality,
  UpdateInfo,
} from "../lib/types";
import { useApp } from "../stores/app";
import { fullWindow, useFullWindow } from "../stores/fullWindow";
import { LyricsChoicesControl } from "../shell/LyricsTrust";
import { PlexEditSettings, plexEditMatches } from "./PlexEditSettings";
import { useHomeUsers } from "../shell/HomeUsers";
import { OutputDeviceSelect } from "../shell/OutputSetting";
import { reloadLikes } from "../stores/likes";
import { loadSettings, updateSettings, useSettings } from "../stores/settings";
import { nav, useCurrentEntry, useRouter } from "../stores/router";
import { ComputerNameSetting } from "./DeviceSettings";
import { INVITE_HOURS, ListenTextSetting } from "./ListenSettings";
import { PlaylistNameSetting } from "./PlaylistSettings";
import { LibraryImportSettings, importMatches } from "./LibraryImportSettings";
import { PerDaySelect } from "./BringIn";
import { AutomationSettings, StreamDeckSettings, automationMatches, streamDeckMatches } from "./StreamDeckSettings";
import { BridgeSettings, NIGHT_HOURS_OPTIONS, bridgeMatches, useBridge } from "./BridgeSettings";
import { MixExclusions } from "./MixExclusions";
import { ArtistAliases } from "./ArtistAliases";
import { DismissedArtists } from "./DiscoverSettings";
import { DefaultLibraries, InvitesToYou, PeopleWithAccess, ShareMyMusic } from "./SharingSettings";
import { DISCOGS_TOKEN_DESCRIPTION, DiscogsImportControl, DiscogsTokenControl, discogsNote, useDiscogs } from "./DiscogsSettings";
import { FolderKeywords, ReleaseStatusTag, ReleaseStatusValues } from "./ArchiveSettings";
import { COVER_PX_OPTIONS, DoctorChecksList, DoctorNote } from "./DoctorSettings";
import { AcoustIdKeyControl, SOUND_ID_DESCRIPTION, SOUND_ID_SCOPE_OPTIONS, soundIdNote, useSoundIdStatus } from "./SoundIdSettings";
import { LibraryCount, LibrarySourceList } from "./LibrarySourceSettings";
import { ServerCount, ServerList } from "./ServerSettings";
import { InfoKeyControl, keyNote, useInfoKeys } from "./ArtistInfoSettings";
import { FeedNote, FeedPortField, useFeedStatus } from "./FeedSettings";
import { CrossfadeControl, EqEditor, PreampControl } from "./AudioSettings";
import { hifiRows } from "./HifiSettings";
import { MoreText } from "./SettingMore";
import { coversNote, discordNote, ImgurKeyControl, imgurNote, useDiscordStatus } from "./DiscordSettings";
import { EXPLICIT_OPTIONS, TagScanNote } from "./FileTagSettings";
import { CheckTagsButton, MusicFolderControl, MusicFolderNote, MusicFolderPath, PlexLibrarySettings } from "./TagEditorSettings";
import { ChangeFolderControl, FolderNote, QueueNote, RemoveAllControl, StorageNote, downloadsTitle, useDownloadStatus } from "./DownloadSettings";
import { DeviceFormatSetting, DeviceListSetting, OpenDevicesButton } from "./DeviceSyncSettings";
import { BatchControl, BatchNote, TranscribeNote } from "./TranscribeSettings";
import { ChangeSyncFolder, KeyCopyButton, SyncNowControl, SyncSwitch, syncNote, useSyncFile } from "./SyncSettings";
import { suggestionLine, useDownloadSuggestions } from "../components/DownloadSuggestions";
import {
  DeviceSyncNowButton,
  ForgetDeviceButton,
  TakeHeldButton,
  deviceLine,
  deviceNote,
  deviceSyncNote,
  useDeviceSync,
} from "./KeepInSyncSettings";
import {
  AiKeyControl,
  AiModelField,
  HiddenCount,
  HiddenList,
  LastfmControl,
  ListenBrainzControl,
  lastfmNote,
  listenbrainzNote,
  useAi,
  useScrobble,
} from "./CurateSettings";
import { loadAi } from "../stores/curate";
import {
  LidarrCheckButton,
  LidarrFindings,
  LidarrHeadersControl,
  LidarrKeyControl,
  LidarrUrlField,
  LastGetLine,
  useLidarr,
} from "./LidarrSettings";
import { LidarrAwayProblem, useHomeSides } from "./LidarrHomeSide";
import { LidarrArtistsList, LidarrChangesList, LidarrWaitingList, LidarrWorkStatus } from "./LidarrModes";
import { LidarrRequestsList } from "./LidarrRequest";
import { OfficialPrefixField, ShareMapEditor, StagingFolderField, UpgradesList } from "./LidarrUpgrade";
import { AUTO_CAP_OPTIONS, AutoArtistsList } from "./AutoRequestDialog";

/** Minutes between Subsonic change checks the setting offers. */
const SUBSONIC_CHECK_OPTIONS = [
  { value: "1", label: "1 minute" },
  { value: "5", label: "5 minutes" },
  { value: "15", label: "15 minutes" },
  { value: "30", label: "30 minutes" },
  { value: "60", label: "1 hour" },
];
import { useTheme } from "../stores/theme";
import { AccentPicker, ThemeFileControl, ThemeFileProblems, ThemeSelect, followNote, themeNote, useThemeFolder } from "./ThemeSettings";
import { LookCards } from "./looks/LookCards";
import { LayoutSettings } from "./looks/LayoutCards";
import { frameInfo } from "../shell/frames";
import { looks } from "../stores/looks";
import { CLASSIC_LAYOUT, identityOf, type Layout } from "../design/themes";
import type { AiProvider, CapName, PlayHistorySource, ReleaseSection, UnreleasedSharing } from "../lib/types";
import {
  HistorySourceControl,
  INCLUDE_KEYS,
  SOURCE_NAMES,
  TautulliKeyControl,
  TautulliUrlField,
  historyNote,
  usePlayHistory,
} from "./PlayHistorySettings";
import { ensureCaps, useCaps, useServerName } from "../stores/caps";
import "./playlist.css";
import { toast } from "../stores/toasts";
import { HelpButton, LearnMore } from "../help/HelpButton";
import {
  DESKTOP_ONLY_PAGES,
  MANAGE_IN_MORE,
  GROUPS,
  GROUP_BLURB,
  GROUP_HELP,
  PAGES,
  advancedView,
  groupId,
  helpFor,
  inMore,
  loadMoreOpen,
  onPhone,
  pageMeta,
  phoneWords,
  resetPatch,
  saveMoreOpen,
  settingsOfRows,
  settingsView,
  type GroupName,
  type PageId,
  type PageMeta,
} from "./settingsMeta";
import { UNRELEASED_RULE_OPTIONS, exceptionsLost, exceptionsTo, patchForRule } from "./unreleasedRule";
import { openContextMenu } from "../components/ContextMenu";
import { GUIDES } from "../guides/guides";
import { GuideButton } from "../guides/GuideButton";
import { failText } from "../lib/errorText";

const errText = (e: unknown) => (e && typeof e === "object" && "message" in e ? String((e as AppError).message) : "");

/** The Layout row's note: the frame, and whether its options were changed. */
function layoutNote(l: Layout | undefined): string {
  const frame = l?.frame ?? "classic";
  const name = frameInfo(frame).name;
  const plain = !l || JSON.stringify({ ...CLASSIC_LAYOUT, ...l }) === JSON.stringify({ ...CLASSIC_LAYOUT, frame });
  return plain ? `${name} layout.` : `${name} layout, with your own options.`;
}

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
const DOWNLOAD_QUALITY = opts<DownloadQuality>(["original", "320", "192", "128"], {
  original: "Original",
  "320": "320 kbps MP3",
  "192": "192 kbps MP3",
  "128": "128 kbps MP3",
});
const DOWNLOAD_LIMITS_GB = [5, 10, 20, 50, 100, 200, 500];
const HOME_QUALITY = opts<StreamQuality>(["original", "320", "256", "192", "128"], QUALITY_LABEL);
const AWAY_QUALITY = opts<StreamQuality>(["320", "original", "192", "128"], QUALITY_LABEL);
const SHUFFLE = [
  { value: "spread", label: "Spread" },
  { value: "standard", label: "Standard" },
] as const satisfies SelectOption<Settings["shuffleStyle"]>[];
const HINT_KEYS = [
  { value: "f", label: "F" },
  { value: "h", label: "H" },
  { value: ";", label: ";" },
  { value: "'", label: "'" },
] as const satisfies SelectOption<Settings["hintKey"]>[];
const AUTOMIX_WHERE = [
  { value: "playlistsAndMixes", label: "Playlists and mixes" },
  { value: "allButAlbums", label: "Everywhere except albums" },
] as const satisfies SelectOption<Settings["automixWhere"]>[];
const AUTOMIX_STYLE = [
  { value: "smooth", label: "Smooth" },
  { value: "energetic", label: "Energetic" },
] as const satisfies SelectOption<Settings["automixStyle"]>[];
const QUEUE_END = [
  { value: "keepPlaying", label: "Keep playing similar music" },
  { value: "stop", label: "Stop" },
] as const satisfies SelectOption<Settings["queueEnd"]>[];
const VARIETY = [
  { value: "balanced", label: "Balanced" },
  { value: "familiar", label: "Familiar" },
  { value: "adventurous", label: "Adventurous" },
] as const satisfies SelectOption<Settings["mixVariety"]>[];
const DAYLIST_HOURS = [
  { value: "1", label: "Every hour" },
  { value: "3", label: "Every 3 hours" },
  { value: "6", label: "Every 6 hours" },
] as const satisfies SelectOption<string>[];
const POPULAR = [
  { value: "global", label: "Global popularity" },
  { value: "myPlays", label: "My plays" },
] as const satisfies SelectOption<PopularSource>[];
const MOTION = [
  { value: "system", label: "Match system" },
  { value: "on", label: "On" },
  { value: "off", label: "Off" },
] as const satisfies SelectOption<ReduceMotion>[];
const RECORD_MOTION = [
  { value: "full", label: "Expanded view" },
  { value: "both", label: "Expanded view and side panel" },
  { value: "off", label: "Off" },
] as const satisfies SelectOption<RecordMotion>[];
const COVER_LIGHT = [
  { value: "on", label: "On" },
  { value: "less", label: "Less" },
  { value: "off", label: "Off" },
] as const satisfies SelectOption<CoverLight>[];
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
/** Languages lyrics can be translated into (language codes the service takes). */
const TRANSLATE_TO: SelectOption<string>[] = [
  ["en", "English"],
  ["es", "Spanish"],
  ["fr", "French"],
  ["de", "German"],
  ["it", "Italian"],
  ["pt", "Portuguese"],
  ["nl", "Dutch"],
  ["sv", "Swedish"],
  ["pl", "Polish"],
  ["tr", "Turkish"],
  ["ru", "Russian"],
  ["uk", "Ukrainian"],
  ["ar", "Arabic"],
  ["hi", "Hindi"],
  ["id", "Indonesian"],
  ["vi", "Vietnamese"],
  ["ja", "Japanese"],
  ["ko", "Korean"],
  ["zh", "Chinese"],
].map(([value, label]) => ({ value, label }));

const DISCORD_STATUS_LINE = [
  { value: "app", label: "Platter" },
  { value: "artist", label: "Artist" },
  { value: "song", label: "Song" },
] as const satisfies SelectOption<Settings["discordStatusLine"]>[];
const DISCORD_COVER_HOST = [
  { value: "auto", label: "Automatic" },
  { value: "litterbox", label: "Litterbox (3 days)" },
  { value: "x0", label: "x0.at (up to 100 days)" },
  { value: "uguu", label: "uguu.se (3 hours)" },
  { value: "catbox", label: "catbox.moe (kept)" },
] as const satisfies SelectOption<Settings["discordCoverHost"]>[];
/** "Cover art source": one choice over Cover Art Archive and uploads (Imgur or a keyless host). */
type CoverSource = "archiveThenUpload" | "archive" | "upload" | "none";
const COVER_SOURCES: SelectOption<CoverSource>[] = [
  { value: "archiveThenUpload", label: "Cover Art Archive, then upload" },
  { value: "archive", label: "Cover Art Archive only" },
  { value: "upload", label: "Upload only" },
  { value: "none", label: "No covers (Platter icon)" },
];
function coverSourceOf(s: Settings): CoverSource {
  const upload = s.discordUploadCovers || s.discordUploadArt;
  if (s.discordCoverArchive) return upload ? "archiveThenUpload" : "archive";
  return upload ? "upload" : "none";
}
function coverSourcePatch(v: CoverSource): Partial<Settings> {
  const archive = v === "archiveThenUpload" || v === "archive";
  const upload = v === "archiveThenUpload" || v === "upload";
  // Turning uploads on uses the keyless hosts; Imgur stays as its own client ID leaves it.
  return upload ? { discordCoverArchive: archive, discordUploadCovers: true } : { discordCoverArchive: archive, discordUploadCovers: false, discordUploadArt: false };
}

/** Under Cover art source: which keyless host uploads go to. */
function CoverHostRow({ s }: { s: Settings }) {
  const id = useId();
  return (
    <div className="setting-sub__row">
      <span id={id} className="setting-sub__label">
        Image host
      </span>
      <Select value={s.discordCoverHost} options={DISCORD_COVER_HOST} onChange={(v) => void updateSettings({ discordCoverHost: v })} labelledBy={id} />
    </div>
  );
}

/** Under Cover art source: your own Imgur client ID, tried before the image host. */
function ImgurRow({ status, setKey }: { status: DiscordStatus | null; setKey: (key: string | null) => Promise<unknown> }) {
  const id = useId();
  return (
    <div className="setting-sub__row" data-setting-part="discordUploadArt">
      <span id={`${id}-t`} className="setting-sub__label">
        Upload to Imgur first
      </span>
      <span id={`${id}-d`} className="sr-only">
        Uses your own Imgur client ID, kept in this computer's keychain.
      </span>
      <span className="setting-sub__control">
        <ImgurKeyControl ids={{ title: `${id}-t`, desc: `${id}-d` }} status={status} setKey={setKey} />
      </span>
      {imgurNote(status) && <span className="setting-sub__note">{errorNote(imgurNote(status))}</span>}
    </div>
  );
}

const ART_SIZES_MB = [512, 1024, 2048, 5120, 10240, 20480];
const LEVELING = [
  { value: "smart", label: "Smart" },
  { value: "track", label: "Song" },
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
export const NOTICES: { name: string; licence: string }[] = [
  { name: "Tauri", licence: "MIT or Apache-2.0" },
  { name: "React", licence: "MIT" },
  { name: "Zustand", licence: "MIT" },
  { name: "TanStack Virtual", licence: "MIT" },
  { name: "Lucide icons", licence: "ISC" },
  { name: "Inter typeface", licence: "SIL Open Font License 1.1" },
  { name: "Atkinson Hyperlegible typeface", licence: "SIL Open Font License 1.1" },
  { name: "Manrope typeface", licence: "SIL Open Font License 1.1" },
  { name: "Fraunces typeface", licence: "SIL Open Font License 1.1" },
  { name: "IBM Plex Mono typeface", licence: "SIL Open Font License 1.1" },
  { name: "Tokio", licence: "MIT" },
  { name: "Serde", licence: "MIT or Apache-2.0" },
  { name: "reqwest", licence: "MIT or Apache-2.0" },
  { name: "rustls", licence: "Apache-2.0, ISC or MIT" },
  { name: "Symphonia", licence: "MPL-2.0" },
  { name: "cpal", licence: "Apache-2.0" },
  { name: "AutoEq (headphone correction profiles)", licence: "MIT" },
  { name: "rusqlite", licence: "MIT" },
  { name: "SQLite", licence: "Public domain" },
  { name: "keyring", licence: "MIT or Apache-2.0" },
  { name: "souvlaki", licence: "MIT" },
  { name: "mdns-sd", licence: "MIT or Apache-2.0" },
  { name: "tokio-rustls", licence: "MIT or Apache-2.0" },
  { name: "RustCrypto MD5", licence: "MIT or Apache-2.0" },
  { name: "RustCrypto SHA-2", licence: "MIT or Apache-2.0" },
  { name: "qrcode (QR codes)", licence: "MIT or Apache-2.0" },
  { name: "Tauri clipboard plugin and arboard (copying images)", licence: "MIT or Apache-2.0" },
  { name: "whisper.cpp and ggml (transcription)", licence: "MIT" },
  { name: "whisper-rs", licence: "Unlicense" },
  { name: "Vulkan headers and Vulkan-Hpp (transcription on the graphics card)", licence: "Apache-2.0 or MIT" },
  { name: "Whisper speech models (downloaded when asked)", licence: "MIT" },
  { name: "rusty-chromaprint (audio fingerprints, a port of Chromaprint)", licence: "MIT" },
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
  { name: "AcoustID (identify by sound, when turned on)", licence: "AcoustID terms, non-commercial use" },
  { name: "Imgur (covers on Discord, with your client ID)", licence: "Imgur API terms" },
  { name: "LRCLIB (lyrics, when turned on)", licence: "LRCLIB terms" },
  { name: "Hugging Face (transcription model download, when asked)", licence: "Hugging Face terms" },
];

// ------------------------------------------------------------------ rows

/** A Settings row note in the error colour (a refused API key). */
function rowNote(n: { text: string; error?: boolean } | undefined): ReactNode {
  if (!n) return undefined;
  return n.error ? errorNote(n.text) : n.text;
}

function errorNote(text: string | undefined): ReactNode {
  return text ? (
    <span className="info-key__error" role="alert">
      {text}
    </span>
  ) : undefined;
}

const AI_PROVIDERS: SelectOption<AiProvider>[] = [
  { value: "none", label: "None" },
  { value: "anthropic", label: "Anthropic Claude" },
];

export interface RowDef {
  key: string;
  /** One of the ten groups (GROUPS). */
  section: GroupName;
  /** The integration or tool page the row lives on, if not the group itself. */
  page?: PageId;
  title: string;
  /** One line (DESCRIPTION_MAX). */
  description: string;
  /** The longer text, behind the row's "?" (searched too). */
  more?: string;
  /** Extra line under the description (state, notes). Not searched. */
  note?: ReactNode;
  /** A separate line below the description. */
  footnote?: string;
  /** A named group inside the section (shown as a small heading). */
  group?: string;
  control: (ids: { title: string; desc: string }) => ReactNode;
  /** A wide part under the row (the EQ editor). */
  block?: (ids: { title: string; desc: string }) => ReactNode;
  /** Shown only when a server in use can do this. */
  cap?: CapName;
  /** False while its service isn't connected or its feature is off: the row waits, and search offers its page. */
  shown?: boolean;
}

function Row({ row, markMore }: { row: RowDef; markMore?: boolean }) {
  const id = useDomId();
  const ids = useMemo(() => ({ title: `${id}-t`, desc: `${id}-d` }), [id]);
  return (
    <div className={row.block ? "setting-row setting-row--wide" : "setting-row"} data-setting={row.key}>
      <RowText
        settingKey={row.key}
        ids={ids}
        title={row.title}
        description={row.description}
        more={row.more}
        note={row.note}
        footnote={row.footnote}
        inMoreMark={markMore && !row.key.startsWith("manage-") && inMore(row)}
      />
      <div className="setting-row__control">{row.control(ids)}</div>
      {row.block && <div className="setting-row__block">{row.block(ids)}</div>}
    </div>
  );
}

/** Where each phone Settings list was scrolled to, by section or page, for this visit. */
const PHONE_SCROLL = new Map<string, number>();

/**
 * A row's description cut to its first sentence, for the phone's one line
 * ("This device only." and other tails go behind the info button).
 */
export function shortLine(text: string): string {
  const m = /^(.+?[.!?])(\s|$)/.exec(text.trim());
  return m ? m[1] : text.trim();
}

/** A row's words. They seldom change, so a change elsewhere on the page skips them. */
const RowText = memo(function RowText({
  settingKey,
  ids,
  title,
  description,
  more,
  note,
  footnote,
  inMoreMark,
}: {
  settingKey: string;
  ids: { title: string; desc: string };
  title: string;
  description: string;
  more?: string;
  note?: ReactNode;
  footnote?: string;
  /** Search results: the row sits under its section's More. */
  inMoreMark?: boolean;
}) {
  const learn = helpFor(settingKey);
  // On a phone each row keeps one short line; the whole explanation is behind its info button.
  const phone = usePhoneForm((x) => x.phone);
  const line = phone ? shortLine(description) : description;
  const info = phone ? [line !== description ? description : null, more].filter(Boolean).join(" ") || undefined : more;
  return (
    <div className="setting-row__text">
      <div className="setting-row__head">
        <div id={ids.title} className="setting-row__title">
          {title}
        </div>
        {inMoreMark && <span className="setting-row__in-more">In More</span>}
        {info && <MoreText title={title} text={info} />}
      </div>
      <p id={ids.desc} className="setting-row__desc">
        {line}
        {note && (
          <>
            {" "}
            <span className="setting-row__note">{note}</span>
          </>
        )}
        {learn && (
          <>
            {" "}
            <LearnMore page={learn[0]} anchor={learn[1]} about={title} />
          </>
        )}
      </p>
      {footnote && <p className="setting-row__desc">{footnote}</p>}
    </div>
  );
});

const loading = <Skeleton width={120} height={32} radius="pill" />;

/** A row's switch for one setting: it follows that setting alone, so changing another leaves it be. */
const KeySwitch = memo(function KeySwitch({ settingKey, ids }: { settingKey: keyof Settings; ids: { title: string; desc: string } }) {
  const on = useSettings((x) => (x.settings ? !!x.settings[settingKey] : null));
  if (on === null) return <Skeleton width={40} height={24} radius="pill" />;
  return <Switch checked={on} onChange={(v) => void updateSettings({ [settingKey]: v })} labelledBy={ids.title} describedBy={ids.desc} />;
});

function toggle(s: Settings | null, key: keyof Settings, ids: { title: string; desc: string }, onChange?: (v: boolean) => void) {
  if (!onChange) return <KeySwitch settingKey={key} ids={ids} />;
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

/** One choice in a row's checklist: a boolean setting, under a short label. */
interface CheckItem {
  key: keyof Settings;
  label: string;
  /** Shown greyed out with this reason. */
  off?: string;
}

/** Several related switches under one row, as a list of checkboxes. */
function Checklist({ s, items, label }: { s: Settings | null; items: CheckItem[]; label: string }) {
  if (!s) return <Skeleton width={240} height={32} />;
  return (
    <fieldset className="setting-checklist">
      <legend className="setting-checklist__legend">{label}</legend>
      {items.map((it) => (
        <label key={it.key} className="setting-checklist__item" data-setting-part={it.key}>
          <input
            type="checkbox"
            checked={!!s[it.key] && !it.off}
            disabled={!!it.off}
            onChange={(e) => void updateSettings({ [it.key]: e.target.checked } as Partial<Settings>)}
          />
          <span>{it.label}</span>
          {it.off && <span className="setting-checklist__off">{it.off}</span>}
        </label>
      ))}
    </fieldset>
  );
}

/** The kinds Get albums starts with, as the choices Settings offers. */
/** Where artist pages' discography opens (the chip picked last is kept the same way). */
const DISCOGRAPHY_CHIP_CHOICES = [
  { value: "popular", label: "Popular releases" },
  { value: "albums", label: "Albums" },
  { value: "singlesEps", label: "Singles and EPs" },
  { value: "compilations", label: "Compilations" },
] as const;

/** The choices, with the one picked last on an artist page when it's another type (Live, one of your own sections...). */
function discographyChipChoices(current: string | undefined): SelectOption<string>[] {
  const known: SelectOption<string>[] = [...DISCOGRAPHY_CHIP_CHOICES];
  if (!current || known.some((c) => c.value === current)) return known;
  const name = current.startsWith("custom:") ? current.slice(7) : (SECTION_LABELS[current as ReleaseSection] ?? current);
  return [...known, { value: current, label: name }];
}

const GET_KIND_CHOICES: { value: string; label: string; kinds: string[] }[] = [
  { value: "studio", label: "Studio albums", kinds: ["studio"] },
  { value: "studio-ep", label: "Studio albums and EPs", kinds: ["studio", "ep"] },
  { value: "studio-ep-live", label: "Studio albums, EPs and live albums", kinds: ["studio", "ep", "live"] },
  { value: "all", label: "Everything but singles", kinds: ["studio", "ep", "live", "compilation"] },
];

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

/** Albums a week a Straight away person gets (same as the core's REQUEST_LIMIT_CHOICES). */
const REQUEST_LIMIT_OPTIONS = [5, 10, 20, 50].map((n) => ({ value: String(n), label: `${n} albums` }));

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

/** Rows with a small heading wherever their group starts. */
/** `markMore`: search results, where rows from under More say so. */
function RowList({ rows, markMore }: { rows: RowDef[]; markMore?: boolean }) {
  // Rows of one small heading stay together, after the rows without one.
  const order = [undefined, ...new Set(rows.map((r) => r.group).filter(Boolean))];
  const sorted = [...rows].sort((a, b) => order.indexOf(a.group) - order.indexOf(b.group));
  return sorted.map((r, i, list) => {
    const starts = r.group && r.group !== list[i - 1]?.group;
    const more = r.group ? GROUP_HELP[r.group] : undefined;
    return (
      <Fragment key={r.key}>
        {starts &&
          (more ? (
            <div className="help-heading">
              <h3 className="settings__subheading">{r.group}</h3>
              <HelpButton page={more[0]} anchor={more[1]} label={`Help with ${r.group}`} />
            </div>
          ) : (
            <h3 className="settings__subheading">{r.group}</h3>
          ))}
        <Row row={r} markMore={markMore} />
      </Fragment>
    );
  });
}

// ------------------------------------------------------------------ page

type UpdateState = { kind: "idle" } | { kind: "checking" } | { kind: "done"; info: UpdateInfo } | { kind: "error"; message: string };

/** Every word of the search is somewhere in the text. */
function matchesText(text: string, q: string): boolean {
  if (!q) return true;
  const hay = text.toLowerCase();
  return q
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean)
    .every((w) => hay.includes(w));
}

/** A row matches on its group, page, heading, title, description or longer text. */
function matches(row: RowDef, q: string): boolean {
  return matchesText(`${row.section} ${pageMeta(row.page)?.title ?? ""} ${row.group ?? ""} ${row.title} ${row.description} ${row.more ?? ""}`, q);
}

/** An integration's card: what it is, whether it's connected, and its page. */
function IntegrationCard({ p, on, onOpen }: { p: PageMeta; on: boolean; onOpen: () => void }) {
  const id = useId();
  return (
    <div className="settings-card" data-page={p.id}>
      <h3 id={id} className="settings-card__title">
        {p.title}
      </h3>
      <p className="settings-card__blurb">{p.blurb}</p>
      <div className="settings-card__foot">
        <span className={`settings-card__status${on ? " is-on" : ""}`}>{on ? "Connected" : "Not set up"}</span>
        <Button size="sm" variant={on ? "secondary" : "primary"} onClick={onOpen} aria-label={`${on ? "Manage" : "Set up"} ${p.title}`}>
          {on ? "Manage" : "Set up"}
        </Button>
      </div>
    </div>
  );
}

/** The group header's ⋯ menu (Reset this section). */
function GroupMenu({ title, onReset }: { title: string; onReset: () => void }) {
  return (
    <button
      type="button"
      className="settings__menu"
      aria-label={`More options for ${title}`}
      aria-haspopup="menu"
      onClick={(e) => openContextMenu(title, [{ label: "Reset this section", icon: RotateCcw, onSelect: onReset }], e.currentTarget)}
    >
      <MoreHorizontal size={18} aria-hidden="true" />
    </button>
  );
}

/** Settings (opened from the header or with Ctrl/Cmd+,). `page` opens one of its pages (tests; the app uses the route). */
export function SettingsPage({ page: pageProp, section: sectionProp }: { page?: PageId; section?: GroupName } = {}) {
  const entry = useCurrentEntry();
  const routePage = entry.route.name === "settings" ? entry.route.page : undefined;
  /** The phone shows one group at a time. */
  const routeSection = entry.route.name === "settings" ? entry.route.section : undefined;
  /** A row a link opened Settings at: its page shows, with its More open. */
  const routeRow = entry.route.name === "settings" ? entry.route.row : undefined;
  const prevRoute = useRouter((x) => x.entries[x.index - 1]?.route);
  const npPrefs = useUi((x) => x.nowPlayingPrefs);
  const setNpPrefs = useUi((x) => x.setNowPlayingPrefs);
  const [bridge] = useBridge();
  const scrobble = useScrobble();
  const ai = useAi();
  const lidarr = useLidarr();
  const homeSides = useHomeSides();
  const history = usePlayHistory();
  const [removeHistory, setRemoveHistory] = useState<PlayHistorySource | null>(null);
  ensureCaps();
  const caps = useCaps((x) => x.all.caps);
  const hasSubsonic = useApp((x) => !!x.servers?.some((v) => v.kind === "subsonic"));
  const serverName = useServerName();
  const s = useSettings((x) => x.settings);
  const [sidebarArt, setSidebarArt] = useSidebarArt();
  const rememberMode = useFullWindow((x) => x.prefs.remember);
  const rememberChips = useLibraryChips((x) => x.remember);
  const auth = useApp((x) => x.auth);
  const healthConn = useApp((x) => x.health?.connection);
  const syncing = useApp((x) => !!x.sync && x.sync.phase !== "idle");
  const [query, setQuery] = useState("");
  const searching = query.trim() !== "";
  const allOpen = advancedView.startOpen;
  /** Sections whose More is open (remembered on this device); with allOpen, the ones closed. */
  const [openAdvanced, setOpenAdvanced] = useState<ReadonlySet<string>>(() => (allOpen ? new Set() : loadMoreOpen()));
  const [resetSection, setResetSection] = useState<string | null>(null);
  /** A sharing rule waiting for an OK, since it undoes exceptions under More. */
  const [pendingRule, setPendingRule] = useState<{ rule: UnreleasedSharing; lost: number } | null>(null);
  const chooseRule = (rule: UnreleasedSharing) => {
    const cur = useSettings.getState().settings;
    const lost = cur ? exceptionsLost(cur, rule).length : 0;
    if (lost > 0) setPendingRule({ rule, lost });
    else void updateSettings(patchForRule(rule, cur));
  };
  const [art, setArt] = useState<ArtCacheInfo | null>(null);
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [update, setUpdate] = useState<UpdateState>({ kind: "idle" });
  const [dialog, setDialog] = useState<"signOut" | "server" | "notices" | null>(null);
  const home = useHomeUsers(true);
  const [rememberHome, setRememberHome] = useState<boolean | null>(null);

  const [accentIssue, setAccentIssue] = useState<string | null>(null);
  const [themeFolder, reloadThemes] = useThemeFolder();
  const theme = useTheme((t) => t.resolved);
  const { keys: infoKeys, setKey: setInfoKey } = useInfoKeys();
  const feed = useFeedStatus();
  const downloads = useDownloadStatus();
  const [syncFile, setSyncFile] = useSyncFile();
  const syncOn = !!s?.syncEnabled && !!s?.syncFolder;
  const [devSync, setDevSync, { failed: devSyncFailed, reload: reloadDevSyncNow }] = useDeviceSync();
  const suggested = useDownloadSuggestions();
  const reloadDevSync = () => void reloadDevSyncNow();
  const devSyncOn = s?.deviceSync !== false;
  const limitGb = s?.downloadLimitGb;
  const limitOptions: SelectOption<string>[] = [...new Set([...DOWNLOAD_LIMITS_GB, ...(limitGb ? [limitGb] : [])])]
    .sort((a, b) => a - b)
    .map((gb) => ({ value: String(gb), label: `${gb} GB` }));
  const { status: discord, setKey: setImgurKey } = useDiscordStatus();
  const { status: soundId, setKey: setAcoustIdKey } = useSoundIdStatus();
  const discogs = useDiscogs();
  const discordOff = !!discord && !discord.available;
  /** Discord's other rows wait until its status is on. */
  const discordOn = !discordOff && !!s?.discordStatus;
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
      (e) => toast.error(failText(`Couldn't clear the cache`, e)),
    );
  const resync = () =>
    backend.syncNow(true).catch((e) => toast.error(failText(`Couldn't start a resync`, e)));
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
      (e) => toast.error(failText(`Couldn't clear artist info`, e)),
    );
  const clearRecentSearches = () =>
    backend.searchRecentsClear().then(
      () => toast.info("Recent searches cleared"),
      (e) => toast.error(failText(`Couldn't clear recent searches`, e)),
    );
  const exportSettings = () =>
    backend.settingsExport().then(
      (path) => path && toast.info("Settings exported"),
      (e) => toast.error(failText(`Couldn't export settings`, e)),
    );
  const importSettings = () =>
    backend.settingsImport().then(
      async (report) => {
        if (!report) return;
        await loadSettings();
        const skipped = report.skipped.length;
        toast.info(
          `Imported ${report.applied} ${report.applied === 1 ? "setting" : "settings"}${skipped ? `. ${skipped} this version can't use ${skipped === 1 ? "was" : "were"} left out` : ""}`,
        );
      },
      (e) => toast.error(failText(`Couldn't import settings`, e)),
    );
  const signOut = () => backend.authSignOut().catch((e) => toast.error(failText(`Couldn't sign out`, e)));

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

  /** A history source's Include switch (with how its import stands) and its Remove row. */
  /** Where each source's Include switch lives: Plex's in Library, the others on their service's page. */
  const HISTORY_HOME: Record<PlayHistorySource, { section: GroupName; page?: PageId }> = {
    plexHistory: { section: "Library" },
    tautulli: { section: "Integrations", page: "tautulli" },
    lastfm: { section: "Integrations", page: "scrobbling" },
  };
  const historyRows = (source: PlayHistorySource, title: string, description: string, more: string): RowDef[] => {
    const st = history.source(source);
    const rows: RowDef[] = [
      {
        key: INCLUDE_KEYS[source],
        ...HISTORY_HOME[source],
        title,
        description,
        more,
        note: historyNote(st),
        control: (ids) => toggle(s, INCLUDE_KEYS[source], ids),
      },
    ];
    if (st && (st.plays > 0 || st.importing || st.error || st.paused)) {
      rows.push({
        key: `remove-${source}`,
        section: "Integrations",
        page: "libraryImport",
        title: `Remove imported plays from ${SOURCE_NAMES[source]}`,
        description: `Deletes only the plays Platter brought in from ${SOURCE_NAMES[source]}. Your own plays in Platter stay.`,
        control: () => <HistorySourceControl history={history} source={source} onRemove={setRemoveHistory} />,
      });
    }
    return rows;
  };
  const isPhone = usePhoneForm((x) => x.phone);
  // The phone keeps each Settings list where you left it: back from a page
  // lands on the row you came from, not at the top.
  const phoneRoot = useRef<HTMLDivElement>(null);
  const phoneScrollKey = useRef("");
  useLayoutEffect(() => {
    if (!isPhone) return;
    const scroller = phoneRoot.current?.closest<HTMLElement>(".ph-page");
    if (!scroller) return;
    const key = phoneScrollKey.current;
    const saved = PHONE_SCROLL.get(key);
    // Rows still arriving can make the list too short at first: try again next frame.
    let frame = 0;
    if (saved) {
      scroller.scrollTop = saved;
      if (scroller.scrollTop < saved) frame = requestAnimationFrame(() => (scroller.scrollTop = saved));
    }
    const save = () => PHONE_SCROLL.set(key, scroller.scrollTop);
    scroller.addEventListener("scroll", save, { passive: true });
    return () => {
      cancelAnimationFrame(frame);
      scroller.removeEventListener("scroll", save);
    };
  }, [isPhone, pageProp, routePage, sectionProp, routeSection]);
  const rows: RowDef[] = [
    // Account
    {
      key: "account",
      section: "Account and servers",
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
      section: "Account and servers",
      title: "Show my Plex profile picture",
      description: "Shows your Plex picture on the app menu button. This device only.",
      more: "Shows your Plex account's picture on the app menu button. Off shows the first letter of your name. This device only.",
      control: (ids) => toggle(s, "showPlexAvatar", ids),
    },
    ...(home.users.length > 1
      ? ([
          {
            key: "homeRemember",
            section: "Account and servers",
            title: "Remember the last Home user",
            description: "Starts as whoever used Platter last. This device only.",
            more: "Platter starts as whoever used it last. Off always starts as the account that signed in. Switch users from the app menu. This device only.",
            control: (ids) => (
              <Switch
                checked={rememberHome ?? home.remember}
                onChange={(v) => {
                  setRememberHome(v);
                  backend.homeRememberSet(v).catch(() => {
                    setRememberHome(!v);
                    toast.error("Couldn't change this setting");
                  });
                }}
                labelledBy={ids.title}
                describedBy={ids.desc}
              />
            ),
          },
        ] satisfies RowDef[])
      : []),
    // Audio quality
    // Setup guides: each opens where it was left.
    ...GUIDES.map(
      (g): RowDef => ({
        key: `guide-${g.id}`,
        section: "App",
        page: "guides",
        title: g.title,
        description: g.summary,
        control: () => <GuideButton id={g.id} />,
      }),
    ),
    {
      key: "qualityHome",
      section: "Sound",
      title: "Streaming quality at home",
      description: "Used on your home network. Original plays the file as it is.",
      more: "Used on your home network. Original plays the file as it is. Changes apply from the next song.",
      note: away ? "You're away from home right now." : undefined,
      control: (ids) => select(s, (x) => x.qualityHome, HOME_QUALITY, ids, (v) => updateSettings({ qualityHome: v })),
    },
    {
      key: "qualityAway",
      section: "Sound",
      title: "Streaming quality away from home",
      description: "Used over remote access or the Plex relay.",
      more: "Used over remote access or the Plex relay. The relay stays capped by the server.",
      note:
        conn === "relay"
          ? "On the relay now, so Plex limits streams to 128 kbps."
          : !away && conn
            ? "You're at home right now."
            : undefined,
      control: (ids) => select(s, (x) => x.qualityAway, AWAY_QUALITY, ids, (v) => updateSettings({ qualityAway: v })),
    },
    {
      key: "downloadQuality",
      section: "Downloads and devices",
      title: "Download quality",
      description: "Original keeps each file as it is; the others are smaller MP3s. This device only.",
      more: "Original keeps each file as it is. The others are MP3s made by your server, which take less space. Applies to new downloads. This device only.",
      control: (ids) => select(s, (x) => x.downloadQuality, DOWNLOAD_QUALITY, ids, (v) => updateSettings({ downloadQuality: v })),
    },
    {
      key: "showFormatBadge",
      section: "Sound",
      title: "Show format badge",
      description: "Shows the format, bit depth and sample rate (like FLAC 24/96) next to songs and albums.",
      control: (ids) => toggle(s, "showFormatBadge", ids),
    },
    {
      key: "signalPathBadge",
      section: "Sound",
      title: "Show the signal path badge",
      description: "A badge in the player bar that shows what happens to the sound on its way out.",
      more: "A badge in the player bar with the playing song's format. Click it to see each step from the file to your output device, whether it's bit-perfect, and what to change if it isn't. The More player options menu and the expanded view open it too.",
      control: (ids) => toggle(s, "signalPathBadge", ids),
    },
    // Playback
    {
      key: "shuffleStyle",
      section: "Playback",
      title: "Shuffle style",
      description: "Spread keeps songs by the same artist apart. Standard is plain random.",
      more: "Spread keeps songs by the same artist apart. Standard is a plain random order.",
      control: (ids) => select(s, (x) => x.shuffleStyle, SHUFFLE, ids, (v) => updateSettings({ shuffleStyle: v })),
    },
    {
      key: "resumeContexts",
      section: "Playback",
      title: "Albums and playlists resume where you left off",
      description: "Playing one again picks up at the song and spot you stopped.",
      more: "Playing an album or playlist again picks up at the song and spot you stopped. Off starts at the first song.",
      control: (ids) => toggle(s, "resumeContexts", ids),
    },
    {
      key: "queueEnd",
      section: "Playback",
      title: "When the queue ends",
      description: "Keep playing similar music from your library once the last song finishes, or stop there.",
      control: (ids) => select(s, (x) => x.queueEnd, QUEUE_END, ids, (v) => updateSettings({ queueEnd: v })),
    },
    {
      key: "smartShuffle",
      section: "Playback",
      title: "Smart shuffle",
      description: "Adds a third shuffle setting that mixes in suggestions from your library.",
      more: "On playlists and Liked Songs, gives the shuffle button a third setting that adds a suggestion from your library after every two songs. Suggestions show a sparkle in the queue.",
      control: (ids) => toggle(s, "smartShuffle", ids),
    },
    {
      key: "precacheCount",
      section: "Playback",
      title: "Songs to load ahead",
      description: "Upcoming songs saved here so they start at once. 1 to 50.",
      more: "How many upcoming songs to save on this computer so they start instantly and survive a dropped connection. 1 to 50.",
      control: (ids) => (s ? <PrecacheField value={s.precacheCount} ids={ids} /> : loading),
    },
    {
      key: "reportPlays",
      section: "Playback",
      title: "Report plays to your server",
      description: `Lets ${serverName ?? "your server"} count plays and show what's playing in your other apps.`,
      control: (ids) => toggle(s, "reportPlays", ids),
    },
    {
      key: "privateSession",
      section: "Sharing and friends",
      title: "Private session",
      description: "Plays aren't saved to your listening history on this device.",
      more: "Plays aren't saved to your listening history on this device. Your server still counts them unless you turn off Report plays to your server, under Playback.",
      control: (ids) => toggle(s, "privateSession", ids),
    },
    {
      key: "outputDevice",
      section: "Sound",
      title: "Output device",
      description: "Where music plays on this computer.",
      more: "Where music plays on this computer. System default follows your computer's sound setting. If the device you pick is unplugged, Platter pauses and switches to the system default.",
      control: (ids) => <OutputDeviceSelect ids={ids} />,
    },
    {
      key: "abLoopHandles",
      section: "Playback",
      title: "Loop part of a song from the seek bar",
      description: "Right-click or Alt+click the seek bar to mark a part that repeats.",
      more: "Right-click or Alt+click the seek bar to mark a start, then an end. The part between them repeats until you clear it with Esc or the A-B chip.",
      control: (ids) => toggle(s, "abLoopHandles", ids),
    },
    // Library
    {
      key: "servers",
      section: "Account and servers",
      title: "Music servers",
      description: "Add Plex, Jellyfin or Subsonic servers and pick their libraries.",
      more: "Add a Plex, Jellyfin, Navidrome or other Subsonic server, choose the primary server, and pick which music libraries come from each. Music on more than one server shows once. Each server's sign-in stays in this computer's keychain. Jellyfin and Plex passwords are never kept; a Subsonic server needs its API key or password on every request, so that one is kept there too. Use an API key where the server offers one.",
      control: () => <ServerCount />,
      block: () => <ServerList />,
    },
    ...(hasSubsonic
      ? ([
          {
            key: "subsonicCheckMinutes",
            section: "Account and servers",
            title: "Check Subsonic servers for changes every",
            description: "How often Platter asks Navidrome and other Subsonic servers for new music.",
            more: "Navidrome and other Subsonic servers can't tell Platter when their music changes, so Platter asks. New music shows up after the next check. Edits to music already in your library arrive with the weekly full sync. This device only.",
            control: (ids) =>
              select(s, (x) => String(x.subsonicCheckMinutes ?? 5), SUBSONIC_CHECK_OPTIONS, ids, (v) => updateSettings({ subsonicCheckMinutes: Number(v) })),
          },
        ] satisfies RowDef[])
      : []),
    {
      key: "saveQueueOnServer",
      section: "Account and servers",
      cap: "resumeQueue",
      title: "Keep my queue on the server",
      description: "Saves what's playing on the server so another app can pick it up.",
      more: "On servers that hold a queue, Platter saves what's playing and what's next as it goes, so another app signed in to the same account can pick it up. A queue another app left there shows under Pick up where you left off, in the player's devices panel.",
      control: (ids) => toggle(s, "saveQueueOnServer", ids),
    },
    {
      key: "libraries",
      section: "Account and servers",
      title: "Servers and libraries",
      description: "Add music from your other Plex servers and ones shared with you.",
      more: "Add music from your other Plex servers and from servers shared with you. With more than one server, the sidebar lets you look at one or all of them, and an album on two servers shows once and plays from the best copy. This device only.",
      control: () => <LibraryCount settings={s} />,
      block: () => <LibrarySourceList settings={s} />,
    },
    {
      key: "popularSource",
      section: "Library",
      title: "Popular songs on artist pages",
      description: "Rank an artist's popular songs by everyone's plays or by your own.",
      more: "Rank an artist's popular songs by how many people play them (from ListenBrainz, or Last.fm top tracks), or by your own plays. With neither source on, your plays are used.",
      control: (ids) => select(s, (x) => x.popularSource, POPULAR, ids, (v) => updateSettings({ popularSource: v })),
    },
    {
      key: "linkFeatured",
      section: "Library",
      title: "Link featured artists",
      description: "Every name in a song's artist line opens a page, featured artists too.",
      more: "Every name in a song's artist line opens a page, including featured artists who have no albums in your library. Their pages show the songs that credit them.",
      control: (ids) => toggle(s, "linkFeatured", ids),
    },
    {
      key: "libraryChipsRemember",
      section: "Library",
      title: "Remember library filters",
      description: "Albums, artists and songs open with the filters you used last. This device only.",
      more: "Albums, artists and songs keep the filter chips you chose last (type, lossless or hi-res, decade, added recently) when you come back or restart Platter. Off, they open unfiltered each time Platter starts. This device only.",
      control: (ids) => (
        <Switch checked={rememberChips} onChange={(v) => useLibraryChips.getState().setRemember(v)} labelledBy={ids.title} describedBy={ids.desc} />
      ),
    },
    {
      key: "downloadsStorage",
      section: "Downloads and devices",
      title: downloadsTitle(downloads?.usedBytes),
      description: "Albums, playlists and Liked Songs kept on this computer so they play without your server.",
      note: <StorageNote />,
      control: () => <RemoveAllControl />,
    },
    ...(suggested.list.map((x) => ({
      key: `downloadSuggestion-${x.kind}-${x.id}`,
      section: "Downloads and devices",
      group: "From your other devices",
      title: x.title,
      description: suggestionLine(x),
      control: () => (
        <>
          <Button size="sm" variant="ghost" onClick={() => suggested.dismiss(x)}>
            Not now
          </Button>
          <Button size="sm" onClick={() => void suggested.download(x)}>
            Download here
          </Button>
        </>
      ),
    })) satisfies RowDef[]),
    {
      key: "downloadLimit",
      section: "Downloads and devices",
      title: "Download storage limit",
      description: "Downloads stop at this size and a banner says so. This device only.",
      more: "Downloads stop at this size and a banner says so. Remove some or raise the limit to carry on. This device only.",
      control: (ids) =>
        s ? (
          <Select
            value={String(s.downloadLimitGb)}
            options={limitOptions}
            onChange={(v) => updateSettings({ downloadLimitGb: Number(v) })}
            labelledBy={ids.title}
            describedBy={ids.desc}
          />
        ) : (
          loading
        ),
    },
    {
      key: "artCache",
      section: "App",
      title: "Artwork and cache size",
      description: "Cover art kept on this computer, up to this size.",
      more: "Cover art is kept on this computer up to this size. Clearing it frees the space; art downloads again as you browse.",
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
      key: "downloadFolder",
      section: "Downloads and devices",
      title: "Download location",
      description: "The folder downloads are kept in. Changing it moves them there. This device only.",
      note: <FolderNote />,
      control: () => <ChangeFolderControl />,
    },
    {
      key: "downloadHomeOnly",
      section: "Downloads and devices",
      title: "Download only on your home network",
      description: "Downloads wait while you're away from home. This device only.",
      more: "Downloads wait while you reach your server over remote access or the Plex relay. This device only.",
      note: <QueueNote />,
      control: (ids) => toggle(s, "downloadHomeOnly", ids),
    },
    {
      key: "downloadLiked",
      section: "Downloads and devices",
      title: "Download Liked Songs",
      description: "Keeps Liked Songs on this computer. Songs you like later download too. This device only.",
      control: (ids) => toggle(s, "downloadLiked", ids),
    },
    {
      key: "goOffline",
      section: "Downloads and devices",
      title: "Go offline",
      description: "Stops using the network; only music on this computer plays. This device only.",
      more: "Platter stops using the network: no server, artist info, lyrics lookups, Discord status, share links or update checks. Only music on this computer plays. Also in the tray menu. This device only.",
      control: (ids) => toggle(s, "goOffline", ids),
    },
    // Devices (Sync to a device)
    {
      key: "devices",
      section: "Downloads and devices",
      page: "devices",
      title: "Devices",
      description: "Drives and players you sync music to. Add one while it's plugged in.",
      more: "USB drives, SD cards and music players you sync music to. Add one on the Devices page while it's plugged in. This device only.",
      control: () => <OpenDevicesButton />,
      block: () => <DeviceListSetting />,
    },
    {
      key: "deviceFormat",
      section: "Downloads and devices",
      page: "devices",
      title: "Default format",
      description: "What songs become on a device that doesn't pick its own format.",
      more: "What songs become on a device that doesn't pick its own format. Original copies each file as it is; the others convert on this computer, in the background.",
      control: (ids) => <DeviceFormatSetting ids={ids} />,
    },
    {
      key: "deviceDeleteRemoved",
      section: "Downloads and devices",
      page: "devices",
      title: "Delete songs removed from the selection",
      description: "The next sync removes songs taken off a device. Other files stay.",
      more: "When you take something off a device, the next sync removes the songs Platter copied for it. Other files on the device are never touched.",
      control: (ids) => toggle(s, "deviceDeleteRemoved", ids),
    },
    // Sync
    {
      key: "syncEnabled",
      section: "App",
      page: "sync",
      title: "Sync with a folder",
      description: "Keeps settings, library folders and lyrics the same on your computers. This device only.",
      more: "Keeps settings, library folders and pins, followed artists and lyrics you wrote the same on your computers, through a folder your cloud app already syncs. Your server keeps likes, playlists and history either way. The file never holds your sign-in or keys. This device only.",
      note: syncNote(s, syncFile),
      control: (ids) => <SyncSwitch s={s} ids={ids} />,
    },
    ...(syncOn
      ? ([
          {
            key: "syncFolder",
            section: "App",
            page: "sync",
            title: "Sync folder",
            description: syncFile?.file ?? s?.syncFolder ?? "",
            control: () => <ChangeSyncFolder />,
          },
          {
            key: "syncNow",
            section: "App",
            page: "sync",
            title: "Sync now",
            description: "Platter syncs a moment after each change and every minute.",
            more: "Platter syncs a moment after each change, when another computer changes the file, and every minute. Changes made while the folder can't be reached go out once it can.",
            control: () => <SyncNowControl status={syncFile} onStatus={setSyncFile} />,
          },
        ] satisfies RowDef[])
      : []),
    // Keep my devices in sync
    {
      key: "deviceSync",
      section: "App",
      page: "sync",
      group: "Keep my devices in sync",
      title: "Keep my devices in sync",
      description: "Your devices on this Plex account swap changes directly, at home and away. This device only.",
      more: "Every Platter signed in to the same Plex account finds the others by itself and swaps changes with them directly: over your network at home, and through the sealed relay when one is away. There's no server, and your sign-in and keys never travel. Your look syncs; each device keeps its own layout, and downloads stay on the device that made them. This device only.",
      note: devSyncFailed ? "Couldn't read sync status." : deviceSyncNote(s, devSync),
      control: (ids) => toggle(s, "deviceSync", ids),
    },
    ...((devSyncFailed
      ? [
          {
            key: "deviceSyncRetry",
            section: "App",
            page: "sync",
            group: "Keep my devices in sync",
            title: "Couldn't read sync status",
            description: "Your paired devices are fine. Try reading it again.",
            control: () => (
              <Button size="sm" onClick={reloadDevSync}>
                Retry
              </Button>
            ),
          },
        ]
      : []) satisfies RowDef[]),
    ...((devSync?.devices ?? []).map((d) => ({
      key: `deviceSync-${d.id}`,
      section: "App",
      page: "sync",
      group: "Keep my devices in sync",
      title: d.name,
      description: deviceLine(devSyncOn ? d : { ...d, words: "Keep my devices in sync is off" }),
      note: deviceNote(d),
      control: () => <ForgetDeviceButton device={d} onOv={setDevSync} />,
    })) satisfies RowDef[]),
    ...((devSync?.devices.length
      ? [
          {
            key: "deviceSyncNow",
            section: "App",
            page: "sync",
            group: "Keep my devices in sync",
            title: "Sync now",
            description: "Platter syncs a moment after each change, and every few minutes.",
            control: () => <DeviceSyncNowButton ov={devSync} on={devSyncOn} onOv={setDevSync} />,
          },
        ]
      : []) satisfies RowDef[]),
    ...((devSync?.quarantined
      ? [
          {
            key: "deviceSyncHeld",
            section: "App",
            page: "sync",
            group: "Keep my devices in sync",
            title: "Changes held back",
            description: `${devSync.quarantined === 1 ? "1 change" : `${devSync.quarantined} changes`} came from a device whose clock is days ahead.`,
            more: "A change stamped more than a day after this device's clock is held back, so a wrong clock can't win over everything you do. Set the other device's clock right, or take them anyway.",
            control: () => <TakeHeldButton onOv={setDevSync} />,
          },
        ]
      : []) satisfies RowDef[]),
    {
      key: "deviceSyncAway",
      section: "App",
      page: "sync",
      group: "Keep my devices in sync",
      title: "Sync when I'm away from home",
      description: "Devices not on this network swap changes through the sealed relay. This device only.",
      more: "When your devices aren't on the same network, changes go through the sealed relay: the relay sees only scrambled messages. Off: devices sync only when they're on the same network, and changes wait until then. This device only.",
      control: (ids) => toggle(s, "deviceSyncAway", ids),
    },
    {
      key: "deviceSyncWifiOnly",
      section: "App",
      page: "sync",
      group: "Keep my devices in sync",
      title: "Send big things only on Wi-Fi",
      description: "A first copy and imported plays wait for Wi-Fi. This device only.",
      more: "On mobile data, Copy from my computer and imported plays wait until you're on Wi-Fi. Small changes, like a pin or a setting, still go. This device only.",
      control: (ids) => toggle(s, "deviceSyncWifiOnly", ids),
    },
    {
      key: "syncSettings",
      section: "App",
      page: "sync",
      group: "What to sync",
      title: "Settings",
      description: "Settings that aren't marked This device only.",
      control: (ids) => toggle(s, "syncSettings", ids),
    },
    {
      key: "syncLibrary",
      section: "App",
      page: "sync",
      group: "What to sync",
      title: "Library folders and pins",
      description: "Your folders, which playlists are in them, and what's pinned in Your Library.",
      control: (ids) => toggle(s, "syncLibrary", ids),
    },
    {
      key: "syncFollows",
      section: "App",
      page: "sync",
      group: "What to sync",
      title: "Followed artists and hidden items",
      description: "Artists you follow, and anything you've hidden or snoozed.",
      control: (ids) => toggle(s, "syncFollows", ids),
    },
    {
      key: "syncLyrics",
      section: "App",
      page: "sync",
      group: "What to sync",
      title: "Lyrics you wrote",
      description: "Lyrics from the lyrics editor, and transcriptions when Sync transcriptions to my devices is on.",
      control: (ids) => toggle(s, "syncLyrics", ids),
    },
    {
      key: "syncLooks",
      section: "App",
      page: "sync",
      group: "What to sync",
      title: "Saved looks and themes",
      description: "The looks and themes you saved or imported. Each device keeps its own layout.",
      control: (ids) => toggle(s, "syncLooks", ids),
    },
    {
      key: "syncDownloads",
      section: "App",
      page: "sync",
      group: "What to sync",
      title: "What each device downloaded",
      description: "Offered on your other devices with one tap. Nothing downloads by itself.",
      control: (ids) => toggle(s, "syncDownloads", ids),
    },
    {
      key: "syncPlays",
      section: "App",
      page: "sync",
      group: "What to sync",
      title: "Imported plays",
      description: "Plays brought in from Plex history, Tautulli, Last.fm or a file. Paired devices only.",
      control: (ids) => toggle(s, "syncPlays", ids),
    },
    {
      key: "keysReceive",
      section: "App",
      page: "sync",
      group: "Keys",
      title: "Copy keys from another device",
      description: "From a computer on the same network and Plex account, with a code.",
      more: "Brings your ListenBrainz, Last.fm, Fanart.tv, TheAudioDB and Imgur keys over from a computer signed in to the same Plex account, on the same network. Shows a code to type on that computer. Keys go straight to this computer's keychain, never through the sync file.",
      control: () => <KeyCopyButton kind="receive" />,
    },
    {
      key: "keysSend",
      section: "App",
      page: "sync",
      group: "Keys",
      title: "Send keys to a device",
      description: "Type the code another of your computers shows, then choose which keys to send it.",
      control: () => <KeyCopyButton kind="send" />,
    },
    {
      key: "resync",
      section: "Library",
      title: "Resync library",
      description: `Reads your whole library from ${serverName ?? "your server"} again.`,
      more: "Use it if something looks out of date. Progress shows at the top.",
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
      section: "Library",
      title: "Show suggestions as you type",
      description: "Completions and matches appear under the search box as you type.",
      more: "Completions and matching artists, albums, songs and playlists appear under the search box while you type.",
      control: (ids) => toggle(s, "searchSuggestions", ids),
    },
    {
      key: "searchTypos",
      section: "Library",
      title: "Forgive typos in search",
      description: "When the words as typed find little, search also shows close spellings, below the exact matches.",
      control: (ids) => toggle(s, "searchTypos", ids),
    },
    {
      key: "searchRecents",
      section: "Library",
      title: "Remember recent searches",
      description: "Things you open from search show when you click the empty box.",
      more: "Things you open from search show up when you click the empty search box. Kept on this computer only.",
      control: (ids) => (
        <>
          {toggle(s, "searchRecents", ids)}
          <Button size="sm" onClick={() => void clearRecentSearches()}>
            Clear recent searches
          </Button>
        </>
      ),
    },
    {
      key: "catalogueSearch",
      section: "Library",
      title: "Show music you don't have in search",
      description: "Search also shows artists, albums and songs you don't have.",
      more: "In with your library's results, artists, albums and songs you don't have, to open, get or buy. Only asked once you stop typing, and only the search text is sent.",
      control: (ids) => toggle(s, "catalogueSearch", ids),
    },
    {
      key: "catalogueSource",
      section: "Library",
      title: "Where music you don't have comes from",
      description: "Lidarr's own search, or MusicBrainz. Songs always come from MusicBrainz.",
      more: "Lidarr's own search is quick and has pictures; MusicBrainz is used when Lidarr isn't set up or can't answer. Songs always come from MusicBrainz.",
      control: (ids) =>
        select(
          s,
          (x) => x.catalogueSource ?? "lidarr",
          [
            { value: "lidarr", label: "Lidarr when it's set up" },
            { value: "musicbrainz", label: "MusicBrainz" },
          ] as const,
          ids,
          (v) => void updateSettings({ catalogueSource: v }),
        ),
    },
    {
      key: "askBeforeGet",
      section: "Library",
      title: "Ask before each Get",
      description: "Show where an album will land and wait for your OK each time.",
      more: "Show where an album will land and wait for your OK each time, instead of sending it at once with a few seconds to undo.",
      control: (ids) => toggle(s, "askBeforeGet", ids),
    },
    {
      key: "getAlbumsKinds",
      section: "Library",
      title: "Album kinds for Get albums",
      description: "What Get albums ticks to start with, and which releases you don't have artist pages show.",
      more: "Which kinds Get albums on an artist's page ticks to start with (you can change them each time). The same kinds decide which releases you don't have show in an artist's discography and full discography: albums, singles and EPs, live albums, compilations.",
      control: (ids) =>
        select(
          s,
          (x) => GET_KIND_CHOICES.find((c) => c.kinds.join(",") === (x.getAlbumsKinds ?? ["studio"]).join(","))?.value ?? "custom",
          [...GET_KIND_CHOICES.map((c) => ({ value: c.value, label: c.label })), ...(GET_KIND_CHOICES.some((c) => c.kinds.join(",") === (s?.getAlbumsKinds ?? ["studio"]).join(",")) ? [] : [{ value: "custom", label: "Your own mix" }])],
          ids,
          (v) => {
            const c = GET_KIND_CHOICES.find((x) => x.value === v);
            if (c) void updateSettings({ getAlbumsKinds: c.kinds });
          },
        ),
    },
    {
      key: "discographyChip",
      section: "Library",
      title: "Discography opens on",
      description: "The release type artist pages show first. The chip you pick last is kept. This device only.",
      control: (ids) =>
        select<string>(s, (x) => x.discographyChip || "popular", discographyChipChoices(s?.discographyChip), ids, (v) => void updateSettings({ discographyChip: v })),
    },
    {
      key: "discographySort",
      section: "Library",
      title: "Full discography order",
      description: "Newest release first, or by name from A to Z. This device only.",
      control: (ids) =>
        select(
          s,
          (x) => x.discographySort ?? "date",
          [
            { value: "date", label: "Release date (newest first)" },
            { value: "name", label: "Name (A to Z)" },
          ] as const,
          ids,
          (v) => void updateSettings({ discographySort: v }),
        ),
    },
    {
      key: "discographyReverse",
      section: "Library",
      title: "Reverse the full discography's order",
      description: "Oldest first, or Z to A. This device only.",
      control: (ids) => toggle(s, "discographyReverse", ids),
    },
    {
      key: "discographyView",
      section: "Library",
      title: "Full discography view",
      description: "Releases with their songs, or a grid of covers. This device only.",
      control: (ids) =>
        select(
          s,
          (x) => x.discographyView ?? "list",
          [
            { value: "list", label: "List" },
            { value: "grid", label: "Grid" },
          ] as const,
          ids,
          (v) => void updateSettings({ discographyView: v }),
        ),
    },
    // Appearance
    {
      key: "look",
      section: "Appearance",
      page: "looks",
      title: "Look",
      description: "Each sets colours, type and surfaces in one click; your layout stays.",
      more: "Ready-made looks. Each one sets colours, type and surfaces in one click; your layout stays as it is.",
      control: () => null,
      block: () => <LookCards />,
    },
    ...(s?.showLookBuilder !== false
      ? [
          {
            key: "makeOwn",
            section: "Appearance",
            page: "looks",
            title: "Make your own",
            description: "Build a look with a live preview, and share it as a file.",
            more: "Build a look with a live preview: colours with contrast checks, corner radius, density, shadows, blur and fonts. Saved looks can be shared as .platter-look files; drop one on the window to add it.",
            control: () => (
              <>
                <Button size="sm" onClick={() => looks.openBuilder("colours")}>
                  Make your own
                </Button>
                <Button size="sm" onClick={() => looks.openPick()}>
                  Pick a look
                </Button>
              </>
            ),
          } satisfies RowDef,
        ]
      : []),
    {
      key: "layout",
      section: "Appearance",
      title: "Layout",
      description: "How the window is arranged: where the player, your sections and the queue go. Synced.",
      more: "Pick a frame: Index puts sections down the left and the player strip across the top, Deck puts a faceplate across the top with source keys under it, Turntable makes the player a column with the record on its platter, Gatefold opens like a record sleeve with now playing on the left page and the page turning on the right, Console gives big libraries a tree, a dense table and a details pane, Classic is the layout Platter had before, and Custom lets you choose what goes where. Turntable's column width and Console's columns, sort and pane widths are kept on this device. Change layout covers the finer options: the player's buttons, Home's shelves, page headers, song list columns, cover shape and motion.",
      note: s ? layoutNote(s.layout) : undefined,
      control: () => (
        <Button size="sm" onClick={() => looks.openBuilder("layout")}>
          Change layout
        </Button>
      ),
      block: () => <LayoutSettings />,
    },
    {
      key: "layoutOffered",
      section: "Appearance",
      title: "Show the layout offer",
      description: "The one-time card offering Index to someone on Classic. Off once it's been seen. This device only.",
      control: (ids) =>
        s ? (
          <Switch checked={!s.layoutOffered} onChange={(v) => void updateSettings({ layoutOffered: !v })} labelledBy={ids.title} describedBy={ids.desc} />
        ) : (
          loading
        ),
    },
    {
      key: "showLookBuilder",
      section: "Appearance",
      page: "looks",
      title: "Show Make your own",
      description: "Shows the look builder here in Appearance and in the first-run look picker.",
      control: (ids) => toggle(s, "showLookBuilder", ids),
    },
    {
      key: "theme",
      section: "Appearance",
      title: "Theme",
      description: "Studio, Studio Black or Sleeve. The older themes are under More looks.",
      more: "Studio is warm graphite with a brass accent, Studio Black puts it on true black for OLED screens, and Sleeve is warm paper and ink. Under More looks: Classic, the look Platter had before, with Classic light, Midnight, Forest and Sand.",
      note: themeNote(s) ?? followNote(s, theme.name),
      control: (ids) => (s ? <ThemeSelect s={s} ids={ids} /> : loading),
    },
    {
      key: "followSystem",
      section: "Appearance",
      title: "Follow system light or dark",
      description: "Switches between a theme's light and dark partner with your computer's mode.",
      more: "When your computer is in light mode a dark theme shows its light partner, and the other way round: Studio and Studio Black pair with Sleeve, the other themes with Classic or Classic light.",
      control: (ids) => toggle(s, "followSystem", ids),
    },
    {
      key: "accent",
      section: "Appearance",
      title: "Accent colour",
      description: "The colour of play buttons, switches and the playing song.",
      more: "The colour of play buttons, switches and the playing song. Brass is Studio's own and Ivory is Classic's. A custom colour needs at least 3:1 contrast with the background.",
      note: rowNote(
        accentIssue
          ? { text: accentIssue, error: true }
          : theme.accentNote
            ? { text: theme.accentNote }
            : theme.fileAccent && (s?.accent === "ivory" || s?.accent === "brass")
              ? { text: "Your theme file sets its own accent. Pick a colour here to replace it." }
              : undefined,
      ),
      control: (ids) => (s ? <AccentPicker s={s} ids={ids} onProblem={setAccentIssue} /> : loading),
    },
    {
      key: "themeFile",
      section: "Appearance",
      page: "looks",
      title: "Your looks and theme files",
      description: "Use a look or theme file from the themes folder.",
      more: "Use a look or theme file from the themes folder. Looks you save, import or drop on the window land there. Files hold only colours, sizes and choices from a fixed list. The one in use travels in the sync file.",
      control: (ids) => (s ? <ThemeFileControl s={s} ids={ids} folder={themeFolder} reload={reloadThemes} /> : loading),
      block: themeFolder?.invalid.length ? () => <ThemeFileProblems folder={themeFolder} /> : undefined,
    },
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
      description: "Turns off sliding and scaling. Fades stay.",
      more: "Turns off movement like sliding and scaling. Fades stay. Match system follows your computer's setting.",
      control: (ids) => select(s, (x) => x.reduceMotion, MOTION, ids, (v) => updateSettings({ reduceMotion: v })),
    },
    {
      key: "recordMotion",
      section: "Appearance",
      title: "Record player",
      description: "A spinning record with a tonearm in now playing.",
      more: "Expanded now playing shows the cover as the label of a record that spins up when music plays and stops on pause, with a tonearm that lowers and lifts. It can turn the side panel's cover too. Reduce motion keeps it still.",
      control: (ids) => select(s, (x) => x.recordMotion, RECORD_MOTION, ids, (v) => updateSettings({ recordMotion: v })),
    },
    {
      key: "tonearmSeek",
      section: "Appearance",
      title: "Tonearm seek bar",
      description: "The seek bar is a stylus on a groove line. It works the same.",
      control: (ids) => toggle(s, "tonearmSeek", ids),
    },
    {
      key: "readoutFace",
      section: "Appearance",
      title: "Readout face",
      description: "Times, formats and counts in a fixed-width face, like a display.",
      more: "Times, formats, bitrates and counts use a fixed-width face, so they read like the display on a piece of gear and never shift as they change. Names and titles stay as they are. Off uses the interface face with even-width figures.",
      note: s && !identityOf(theme.id) ? "Only applies in Studio, Studio Black and Sleeve." : undefined,
      control: (ids) => toggle(s, "readoutFace", ids),
    },
    {
      key: "coverLight",
      section: "Appearance",
      title: "Cover light",
      description: "The playing cover's colour as a soft light behind now playing.",
      more: "The cover of the song playing lights the player area a little, like a lamp behind it: a soft pool and a lit edge, never on the text. It changes only when the song does. Less makes it fainter.",
      note:
        s && !identityOf(theme.id)
          ? "Only applies in Studio, Studio Black and Sleeve."
          : s && !s.artworkColours
            ? "Turn on Artwork colours to use this."
            : undefined,
      control: (ids) => select(s, (x) => x.coverLight, COVER_LIGHT, ids, (v) => updateSettings({ coverLight: v })),
    },
    {
      key: "upNextLine",
      section: "Appearance",
      title: "Show what's next in the player bar",
      description: "Near a song's end, the line under its title shows the next song.",
      more: "In the last seconds of a song, the line under its title shows the next song. Click it to open the queue.",
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
      section: "Appearance",
      title: "Show the shortcut grid",
      description: "Shows up to 8 things you've played lately or play most at the top of Home.",
      control: (ids) => toggle(s, "homeShortcuts", ids),
    },
    {
      key: "homeRecordsFirst",
      section: "Appearance",
      title: "Home leads with records",
      description: "In Studio, Studio Black and Sleeve, Home opens with the album you were in the middle of.",
      more: "Home starts with the album you stopped partway through, ready to resume, then New arrivals, Spun this week, Finish what you started and Not played in a year. Mixes, playlists and the other shelves follow. Classic and the other looks keep the usual Home.",
      note: s && !identityOf(theme.id) ? "Only applies in Studio, Studio Black and Sleeve." : undefined,
      control: (ids) => toggle(s, "homeRecordsFirst", ids),
    },
    {
      key: "homeColourWash",
      section: "Appearance",
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
    { key: "homeOnThisDay", section: "Appearance", title: "Show On this day on Home", description: "What you played on today's date in earlier years.", control: (ids) => toggle(s, "homeOnThisDay", ids) },
    { key: "homeTimeTravel", section: "Appearance", title: "Show Time travel on Home", description: "A mix of your most played songs from each decade, and your year.", control: (ids) => toggle(s, "homeTimeTravel", ids) },
    // Mixes
    {
      key: "homeMixes",
      section: "Library",
      title: "Show made-for-you mixes on Home",
      description: "Daily Mixes, the daylist and Discover, made on this computer.",
      more: "Daily Mixes, the daylist, Discover and your mood and genre mixes, made on this computer from what you play.",
      control: (ids) => toggle(s, "homeMixes", ids),
    },
    {
      key: "mixVariety",
      section: "Library",
      title: "Mix variety",
      description: "Familiar sticks mostly to songs you play a lot. Adventurous brings in more you haven't heard much.",
      control: (ids) => select(s, (x) => x.mixVariety, VARIETY, ids, (v) => updateSettings({ mixVariety: v })),
    },
    {
      key: "mixExclude",
      section: "Library",
      title: "Leave out of mixes",
      description: "Artists, albums and genres that never go in mixes or radio.",
      control: (ids) => (s ? <MixExclusions settings={s} ids={ids} /> : loading),
    },
    {
      key: "mixUnreleased",
      section: "Sharing and friends",
      group: "Unreleased music, feature by feature",
      title: "Include unreleased music in mixes",
      description: "Demos and other unreleased versions can be in mixes and radio.",
      control: (ids) => toggle(s, "mixUnreleased", ids),
    },
    {
      key: "daylistHours",
      section: "Library",
      title: "Refresh the daylist",
      description: "How often the daylist changes to fit the time of day.",
      control: (ids) => select(s, (x) => String(x.daylistHours), DAYLIST_HOURS, ids, (v) => updateSettings({ daylistHours: Number(v) })),
    },
    {
      key: "hiddenItems",
      section: "Library",
      page: "hidden",
      title: "Hidden songs and artists",
      description: "Mixes, radio and shuffle skip these. They still play when you pick them.",
      more: "Mixes, radio, Keep playing similar music and shuffle play skip these. They still play when you pick them yourself. Kept in the sync file when Followed artists and hidden items is synced.",
      control: () => <HiddenCount />,
      block: () => <HiddenList />,
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
      key: "notifications",
      section: "App",
      title: "Notifications",
      description: "What Platter tells you about. Never during a private session.",
      more: "New releases: a desktop notification when an artist you follow gets new music in your library. Release radar: when it finds something new. The bell: a dot when artists you follow have new releases in your library. Song changes: a desktop notification when a new song starts, unless Platter is in front or you're skipping through songs (this device only). Who you follow is kept on this computer.",
      control: () => null,
      block: () => (
        <Checklist
          s={s}
          label="Tell me about"
          items={[
            { key: "notifyNewReleases", label: "New releases from artists I follow" },
            {
              key: "radarNotify",
              label: "What the release radar finds",
              off: s?.releaseRadar === false ? "Turn on Find new releases under Library to use this." : undefined,
            },
            { key: "whatsNewAnnouncements", label: "New releases, as a dot on the bell" },
            { key: "notifyTrackChange", label: "Each new song (this device only)" },
          ]}
        />
      ),
    },
    {
      key: "mediaSession",
      section: "App",
      title: "Show in the system media controls",
      description: "Song, artist and cover in your computer's media controls and lock screen.",
      more: platform === "mac"
          ? "Shows the song, artist and cover in Control Center and on the lock screen, with play, pause, next and previous."
          : "Shows the song, artist and cover in your computer's media controls and on the lock screen, with play, pause, next and previous.",
      control: (ids) => toggle(s, "mediaSession", ids),
    },
    {
      key: "mediaKeys",
      section: "App",
      title: "Media keys",
      description: "Play, next and previous keys control Platter, even in the background.",
      more: "The play, next and previous keys on your keyboard or headset control Platter, even when it's in the background.",
      note: s?.mediaSession ? "While Platter is in the system media controls, your computer sends these keys there." : undefined,
      control: (ids) => toggle(s, "mediaKeys", ids),
    },
    {
      key: "closeToTray",
      section: "App",
      title: `Close to the ${TRAY}`,
      description: `Closing the window keeps Platter playing in the ${TRAY}.`,
      more: `Closing the window keeps Platter playing in the ${TRAY}. Click its icon to bring the window back, or quit from its menu.`,
      control: (ids) => toggle(s, "closeToTray", ids),
    },
    {
      key: "updateByItself",
      section: "App",
      title: "Update by itself",
      description: "New versions install when Platter opens or when you quit. Never while music plays.",
      more: "Platter checks a few seconds after it opens and every few hours. An update found right after opening installs straight away if nothing is playing, with a moment to say Not now. Otherwise it waits and installs when you quit. Off, you choose when to restart.",
      control: (ids) => toggle(s, "updateByItself", ids),
    },
    {
      key: "launchAtLogin",
      section: "App",
      title: "Open Platter when you sign in",
      description: "Starts Platter when you sign in to your computer.",
      more: `It starts hidden in the ${TRAY} if Close to the ${TRAY} is on, otherwise minimised.`,
      control: (ids) => toggle(s, "launchAtLogin", ids),
    },
    {
      key: "keyboardShortcuts",
      section: "App",
      title: "Keyboard shortcuts",
      description: `Every shortcut, for playback, getting around and the layout. ${shortcutLabel("help")} opens the list from anywhere.`,
      control: () => (
        <Button size="sm" onClick={() => useUi.getState().setShortcutsOpen(true)}>
          Show shortcuts
        </Button>
      ),
    },
    {
      key: "powerKeys",
      section: "App",
      title: "Keyboard power mode",
      description: "Labels everything you can click with two letters; type them to use it.",
      more: "Press the hint key to label everything you can click with two letters, then type the letters to use it. J and K move through songs in a list, G G jumps to the top and Shift+G to the bottom, Enter plays and / goes to search. Main window only.",
      control: (ids) => toggle(s, "powerKeys", ids),
    },
    {
      key: "hintKey",
      section: "App",
      title: "Key that shows jump hints",
      description: "The key that shows the jump hints while keyboard power mode is on.",
      control: (ids) => select(s, (x) => x.hintKey, HINT_KEYS, ids, (v) => updateSettings({ hintKey: v })),
    },
    {
      key: "showSetupGuides",
      section: "App",
      title: "Show setup guides for new features",
      description: "Offers a short guide when a feature has one.",
      more: "Offers a short guide the first time you start Platter and when a feature has one. Setup guides, under App, always has them.",
      control: (ids) => toggle(s, "showSetupGuides", ids),
    },
    {
      key: "showHelpButtons",
      section: "App",
      title: "Show \"?\" help buttons",
      description: "Small \"?\" buttons next to busier screens open their help page.",
      more: "Small \"?\" buttons next to busier screens open their help page beside them. Help is also in your profile menu and in search.",
      control: (ids) => toggle(s, "showHelpButtons", ids),
    },
    {
      key: "settingsFile",
      section: "App",
      title: "Export or import settings",
      description: "All your settings in one file, never with keys or passwords.",
      more: "Saves all your settings in one file to keep or to open on another computer. Keys and passwords are never in it; they stay in this computer's keychain. Folders and devices stay as they are on each computer.",
      control: () => (
        <>
          <Button size="sm" onClick={() => void exportSettings()}>
            Export
          </Button>
          <Button size="sm" onClick={() => void importSettings()}>
            Import
          </Button>
        </>
      ),
    },
    // About
    {
      key: "about",
      section: "App",
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
      description: "Asks before adding a song a playlist already has.",
      more: "When a song you add from a menu or by dragging is already in that playlist, ask whether to add it again.",
      control: (ids) => toggle(s, "playlistAskDuplicates", ids),
    },
    {
      key: "playlistNewName",
      section: "Library",
      title: "New playlists start with name",
      description: "#N becomes the next free number: My playlist #1, #2 and so on.",
      more: "The name a new playlist gets. #N becomes the next free number, so you get My playlist #1, #2 and so on.",
      control: (ids) => <PlaylistNameSetting ids={ids} />,
    },
    // Library: likes
    {
      key: "likedMeans",
      section: "Library",
      // Only where likes are star ratings (Plex); elsewhere a like is a like.
      cap: "starRatings",
      title: "Liked means",
      description: "Which Plex star ratings count as liked. Liking a song gives it five stars.",
      more: "Liked Songs are Plex star ratings, so other Plex apps see them. Liking a song always gives it five stars; this picks which ratings count as liked.",
      control: (ids) => select(s, (x) => x.likedMeans, LIKED_MEANS, ids, (v) => void updateSettings({ likedMeans: v }).then(reloadLikes)),
    },
    {
      key: "bookmarksInLibrary",
      section: "Library",
      title: "Show bookmarks in Your Library",
      description: "A Bookmarks row under Liked Songs. Bookmark the playing song with Alt+Shift+M.",
      more: "A Bookmarks row under Liked Songs lists the songs and pages you bookmarked. Bookmark the playing song with Alt+Shift+M.",
      control: (ids) => toggle(s, "bookmarksInLibrary", ids),
    },
    {
      key: "pinsInLibrary",
      section: "Library",
      title: "Show pins at the top of Your Library",
      description: "Up to 4 pinned items stay above everything else.",
      more: "Up to 4 playlists, albums, artists or folders you pin stay above everything else. Pin one from its menu. Pins sync with your other devices.",
      control: (ids) => toggle(s, "pinsInLibrary", ids),
    },
    {
      key: "spineWidthByTracks",
      section: "Library",
      title: "Spine width follows track count",
      description: "In the Shelf view, albums with more songs stand thicker. Saved on this computer.",
      more: "Your Library's Shelf view stands albums as spines. With this on, a spine is wider the more songs the album has, so a double album reads thicker than a single. Off, every spine is the same width. Saved on this computer only.",
      control: (ids) => toggle(s, "spineWidthByTracks", ids),
    },
    // Library: stats
    {
      key: "statsPage",
      section: "Library",
      title: "Show the Stats page",
      description: "Your listening in numbers, counted on this computer.",
      more: "Your listening in numbers: minutes, top songs, artists, albums and genres for any range of days. Opens from your profile menu and a Stats row in Your Library. Your stats in a song's, artist's or album's menu shows your plays, minutes and rank for it. Compare shares only a summary of your top artists and songs, never your history. Wrapped is your year in cards from December 1, with a Your Top Songs list for each year with 30 plays or more.",
      control: (ids) => toggle(s, "statsPage", ids),
      block: () => (
        <Checklist
          s={s}
          label="Also show"
          items={[
            { key: "monthlyStats", label: "Monthly recaps" },
            { key: "statsCompare", label: "Compare stats with friends" },
            { key: "wrappedTopSongs", label: "Wrapped and yearly Top Songs" },
            {
              key: "topSongsShowPlays",
              label: "Plays on yearly Top Songs lists",
              off: s?.wrappedTopSongs === false ? "Turn on Wrapped and yearly Top Songs to use this." : undefined,
            },
          ]}
        />
      ),
    },
    {
      key: "artistAliases",
      section: "Library",
      title: "Same artist under two names",
      description: "Names matched to one artist count as one in Stats. Join others here.",
      more: "An artist who changed their name can show up twice. With this on, names your server matched to the same MusicBrainz artist are counted as one on their own, and Not the same artist keeps a pair apart. Join any others by hand below. Their plays add up under the name you played most. The switch syncs with your look and settings; the names you join or keep apart are kept in the sync file when Followed artists and hidden items is synced.",
      control: (ids) => toggle(s, "artistAliasesAuto", ids),
      block: (ids) => <ArtistAliases ids={ids} />,
    },
    // Library: the listening diary
    {
      key: "keepDiary",
      section: "Library",
      title: "Keep a listening diary",
      description: "A private journal of albums you play start to finish, to rate and write about.",
      more: "When you play an album from start to finish, a dated entry goes in your diary with how much you played and where: this computer, and headphones or speakers when the output's name says. Add a rating and a short note, or log any album from its menu. Album pages show when you first heard them. The diary is never shared or scrobbled, and stays on this computer unless you sync it.",
      control: (ids) => toggle(s, "keepDiary", ids),
      block: () =>
        s?.keepDiary === false ? null : (
          <Checklist
            s={s}
            label="Diary options"
            items={[
              { key: "diaryAuto", label: "Add albums played start to finish automatically" },
              { key: "syncDiary", label: "Sync the diary to my devices" },
            ]}
          />
        ),
    },
    // Library: stats from your listening before Platter and in other apps
    ...historyRows("plexHistory", "Include my Plex listening history", "Stats and mixes also count what you played in Plex before Platter.", "Stats, mixes, On Repeat and recaps also count what you played on your Plex server before Platter and in other Plex apps. Only your own plays, never other people's on the server, and a play from Platter counts once."),
    ...(history.source("tautulli")?.setUp
      ? historyRows("tautulli", "Include Tautulli history", "Stats and mixes also count your music plays in Tautulli.", "Stats, mixes, On Repeat and recaps also count your music plays in Tautulli. Plays too short to count are left out, and a play Plex or Platter already has counts once.")
      : []),
    ...(history.source("lastfm")?.setUp
      ? historyRows("lastfm", "Include Last.fm scrobbles", "Stats and mixes also count your Last.fm scrobbles.", "Stats, mixes, On Repeat and recaps also count your Last.fm scrobbles, matched to your library by artist, song and album. Songs you don't have still count for their artists. Platter's own scrobbles count once.")
      : []),
    // Library: the physical collection
    {
      key: "showPhysicalCollection",
      section: "Library",
      title: "Show physical collection",
      description: "Mark albums you own on vinyl, CD or cassette, and see them on your shelf.",
      more: "Mark albums you own on vinyl, CD, cassette or another format from an album's menu, with notes on each copy. Shows an On vinyl badge on the album, an On the shelf filter in Your Library, and your shelf page with its numbers.",
      control: (ids) => toggle(s, "showPhysicalCollection", ids),
    },
    {
      key: "shelfMissingList",
      section: "Library",
      title: "Show what's missing from the shelf",
      description: "Lists the albums you play most that you don't own, on your shelf page.",
      more: "Lists the albums you play most that you don't own on any format, on your shelf page.",
      control: (ids) => (s?.showPhysicalCollection === false ? null : toggle(s, "shelfMissingList", ids)),
    },
    {
      key: "shelfOpen",
      section: "Library",
      title: "Your shelf",
      description: "Every album you own, what a Discogs import found that isn't in your library, and what's missing.",
      control: () =>
        s?.showPhysicalCollection === false ? null : (
          <Button size="sm" onClick={() => nav.go({ name: "shelf" })}>
            Open your shelf
          </Button>
        ),
    },
    // Library: sleeve notes
    {
      key: "showSleeveNotes",
      section: "Library",
      title: "Show sleeve notes",
      description: "A Sleeve notes tab on album pages, and Credits in every song's menu.",
      more: "Album pages get a Sleeve notes tab next to the songs: who produced, wrote, played on, engineered, mixed and mastered each song, when and where it was recorded, the label, catalogue number, country, barcode, format and edition, and the release's own notes. Each fact is marked with where it came from: your files' tags, Plex, MusicBrainz or Discogs. Names open a page with everything in your library they're credited on.",
      control: (ids) => toggle(s, "showSleeveNotes", ids),
    },
    {
      key: "sleeveLookups",
      section: "Library",
      title: "Look up credits online",
      description: "Sleeve notes ask MusicBrainz, and Discogs once you've added a token.",
      more: "When you open an album's sleeve notes, Platter looks the release up on MusicBrainz (by its MusicBrainz id, or its artist and title when there's none) and, if you've added a Discogs token, on Discogs. Answers are kept on this computer, so offline you still see what was found. Unreleased music is never looked up. Off: only your files' tags and Plex.",
      control: (ids) => (s?.showSleeveNotes === false ? null : toggle(s, "sleeveLookups", ids)),
    },
    {
      key: "showVinylSides",
      section: "Library",
      title: "Show vinyl sides",
      description: "Album songs are grouped into Side A, Side B and so on, with Play side.",
      more: "When an album has a vinyl edition on MusicBrainz or Discogs, its songs are grouped by the record's sides, and each side can be played on its own from its heading or the album's menu. Any album can be split by hand with Split into sides in its menu; splits are kept for every device you sync. The album still plays in order.",
      control: (ids) => toggle(s, "showVinylSides", ids),
    },
    // Library: crates
    {
      key: "showCrates",
      section: "Library",
      title: "Show crates",
      description: "A Crates page in Explore on Home: your records by label, producer, studio, era and genre.",
      more: "Crates file your records the way a shop would. Labels come from your files' tags, Plex and the sleeve notes you've opened; producers from credit tags and sleeve notes; studios from sleeve notes looked up online. A crate needs two records or more. Each crate's page plays everything in it, and a record that's on the sound map opens there.",
      control: (ids) => toggle(s, "showCrates", ids),
    },
    {
      key: "crateShelves",
      section: "Library",
      title: "Crate shelves on album pages",
      description: "Album pages end with more from the same label, producer and studio.",
      more: "Under the songs, an album page shows up to two shelves such as More from its label and More produced by its producer, from your library only. Any other crates it's in are listed under More crates.",
      control: (ids) => (s?.showCrates === false ? null : toggle(s, "crateShelves", ids)),
    },
    {
      key: "likeControlRows",
      section: "Library",
      title: "Show the like control on song rows",
      description: "Show the plus on hover, on every row, or not in rows.",
      more: "On hover shows the plus when you point at a song (songs in Liked Songs always show their tick). Always shows it on every row; Off hides it in rows.",
      control: (ids) => select(s, (x) => x.likeControlRows, LIKE_ROWS, ids, (v) => updateSettings({ likeControlRows: v })),
    },
    // Integrations: artist info
    {
      key: "artistInfoSources",
      section: "Integrations",
      page: "artistInfo",
      title: "Free sources",
      description: "Only an artist's or album's name or MusicBrainz id is sent.",
      more: "Wikipedia and Wikidata: longer bios, where an artist is from, years active and band members. MusicBrainz: exact release dates and credits on album pages and in song credits, band members and related artists. ListenBrainz: ranks an artist's Popular songs by how many people listen, and shows listener numbers when Last.fm is off or has none. For featured artists their name goes to MusicBrainz to find their id.",
      control: () => null,
      block: () => (
        <Checklist
          s={s}
          label="Use these sources"
          items={[
            { key: "infoWikipedia", label: "Wikipedia and Wikidata (bios and facts)" },
            { key: "infoMusicbrainz", label: "MusicBrainz (release dates, credits and members)" },
            { key: "infoListenbrainz", label: "ListenBrainz (popular songs)" },
            { key: "infoListenbrainzStats", label: "ListenBrainz (listener numbers)" },
          ]}
        />
      ),
    },
    {
      key: "infoLastfm",
      section: "Integrations",
      page: "artistInfo",
      title: "Last.fm",
      description: "Listener counts, tags and a bio. Needs your own free Last.fm API key.",
      more: "Listener counts, tags and a bio when Wikipedia has none. Needs your own free Last.fm API key, kept in this computer's keychain.",
      note: errorNote(keyNote("lastfm", infoKeys)),
      control: (ids) => <InfoKeyControl source="lastfm" ids={ids} keys={infoKeys} setKey={setInfoKey} />,
    },
    {
      key: "infoFanart",
      section: "Integrations",
      page: "artistInfo",
      title: "Fanart.tv",
      description: "Banners, logos and photos Plex has no art for. Needs your Fanart.tv key.",
      more: "Banners, logos and photos for artists Plex has no art for. Needs your Fanart.tv API key, kept in this computer's keychain.",
      note: errorNote(keyNote("fanart", infoKeys)),
      control: (ids) => <InfoKeyControl source="fanart" ids={ids} keys={infoKeys} setKey={setInfoKey} />,
    },
    {
      key: "infoAudiodb",
      section: "Integrations",
      page: "artistInfo",
      title: "TheAudioDB",
      description: "A photo and bio when nothing else has one. Needs your own key.",
      more: "A photo and bio when nothing else has one. Needs your own TheAudioDB key; its shared test key isn't for use in apps.",
      note: errorNote(keyNote("audiodb", infoKeys)),
      control: (ids) => <InfoKeyControl source="audiodb" ids={ids} keys={infoKeys} setKey={setInfoKey} />,
    },
    {
      key: "infoListenbrainzToken",
      section: "Integrations",
      page: "artistInfo",
      title: "ListenBrainz token",
      description: "Optional. Some artists need a ListenBrainz user token.",
      more: "Optional. ListenBrainz answers some artists only with a user token. Copy yours from your ListenBrainz settings page; it's kept in this computer's keychain.",
      note: errorNote(keyNote("listenbrainz", infoKeys)),
      control: (ids) => <InfoKeyControl source="listenbrainz" ids={ids} keys={infoKeys} setKey={setInfoKey} />,
    },
    {
      key: "infoCache",
      section: "Integrations",
      page: "artistInfo",
      title: "Clear artist info cache",
      description: "Artist info is kept for 30 days. Clearing it fetches it again as you browse.",
      more: "Artist info is kept on this computer for 30 days. Clearing it fetches everything again as you open artists.",
      control: () => (
        <Button size="sm" onClick={() => void clearArtistInfo()}>
          Clear artist info
        </Button>
      ),
    },
    // Integrations: Discogs
    {
      key: "discogsToken",
      section: "Integrations",
      page: "discogs",
      title: "Discogs token",
      description: "A personal access token from Discogs, kept in this computer's keychain.",
      more: DISCOGS_TOKEN_DESCRIPTION,
      note: discogsNote(discogs),
      control: (ids) => <DiscogsTokenControl ids={ids} st={discogs} />,
    },
    {
      key: "discogsImport",
      section: "Integrations",
      page: "discogs",
      title: "Import your collection",
      description: "Marks albums in your Discogs collection and lists the ones you don't have.",
      more: "Marks the albums in your library that are in your Discogs collection, and lists the records that aren't with ways to request or buy them. Read it again any time; copies you edited keep your notes. Data provided by Discogs.",
      shown: !!discogs?.saved,
      control: () => <DiscogsImportControl st={discogs} />,
    },
    // Integrations: the now-playing feed for widgets
    {
      key: "nowPlayingFeed",
      section: "Integrations",
      page: "widgetFeed",
      title: "What's playing, for desktop widgets",
      description: "Serves what's playing to widgets on this computer only. This device only.",
      more: "Serves what's playing to OBS, Rainmeter and other widgets on this computer, as data and as an overlay page. Only this computer can reach it, never your network. A private session blanks it. This device only.",
      note: s?.nowPlayingFeed ? <FeedNote status={feed} /> : undefined,
      control: (ids) => toggle(s, "nowPlayingFeed", ids),
    },
    {
      key: "nowPlayingFeedPort",
      section: "Integrations",
      page: "widgetFeed",
      title: "Feed port number",
      description: "The port on 127.0.0.1 the feed uses. This device only.",
      more: "The port on 127.0.0.1 the feed uses, from 1024 to 65535. Change it if another program has it. This device only.",
      shown: !!s?.nowPlayingFeed,
      control: (ids) => (s ? <FeedPortField value={s.nowPlayingFeedPort} ids={ids} /> : loading),
    },
    // Downloads and devices: listen on
    {
      key: "showDevices",
      section: "Downloads and devices",
      title: "Show these devices",
      description: "What Listen on lists. Off stops Platter looking for them.",
      more: "Plex players: Plexamp, Plex apps and Sonos speakers linked to Plex. Chromecast and AirPlay: Chromecasts, Google speakers and groups, and AirPlay speakers on your network. DLNA: receivers, smart speakers and TVs that play over DLNA or UPnP, straight from your server (the original file when the device takes its format, otherwise a version your server converts). AirPlay devices that need a password or pairing code ask for it when you pick one.",
      control: () => null,
      block: () => (
        <Checklist
          s={s}
          label="Show these devices"
          items={[
            ...(caps.remotePlayers ? [{ key: "showPlexPlayers" as const, label: "Plex players (Plexamp, Plex apps, Sonos)" }] : []),
            { key: "showCastDevices", label: "Chromecast and AirPlay" },
            { key: "showDlnaDevices", label: "Other network speakers and TVs (DLNA)" },
            { key: "showAirplayLocked", label: "AirPlay devices that need a password" },
          ]}
        />
      ),
    },
    {
      key: "moveStopsOldDevice",
      section: "Downloads and devices",
      title: "Stop on the old device when moving playback",
      description: "Moving playback pauses it on the device it left.",
      more: "Moving playback to another device pauses it here, and bringing it back stops it there. Off leaves both playing.",
      control: (ids) => toggle(s, "moveStopsOldDevice", ids),
    },
    {
      key: "actAsPlexPlayer",
      section: "Downloads and devices",
      cap: "remotePlayers",
      title: "Let other Plex apps play on this computer",
      description: "Shows Platter as a player in Plexamp and other Plex apps. This device only.",
      more: "Platter shows up as a player in Plexamp and other Plex apps on your network, so they can send music here. Opens a network port, so your computer may ask to allow it. This device only.",
      control: (ids) => toggle(s, "actAsPlexPlayer", ids),
    },
    {
      key: "showToMyDevices",
      section: "Downloads and devices",
      title: "Show this computer to my other devices",
      description: "Your other devices running Platter can see this computer and play on it. This device only.",
      more: "Your other devices running Platter on this network, signed in to the same Plex account, can see this computer and play on it. Opens a network port, so your computer may ask to allow it. This device only.",
      control: (ids) => toggle(s, "showToMyDevices", ids),
    },
    {
      key: "computerName",
      section: "Downloads and devices",
      title: "This computer's name",
      description: "What your other devices call this computer. This device only.",
      more: "What your other devices call this computer. Leave it empty to use the computer's own name. This device only.",
      control: (ids) => <ComputerNameSetting ids={ids} />,
    },
    // On this phone (the phone layout only)
    ...(isPhone ? phoneRows(s) : []),
    // Visualizer
    ...visualizerRows(s, isPhone),
    // Transitions
    {
      key: "crossfade",
      section: "Playback",
      title: "Crossfade songs",
      description: "Smart overlaps only the quiet end and start. Fixed fades over the time you set.",
      more: "Smart overlaps only the quiet end of a song and the quiet start of the next, so songs that end cold aren't cut and fades keep their shape. Fixed fades over the time you set. Skipping always cuts straight to the next song.",
      control: (ids) => <CrossfadeControl ids={ids} />,
    },
    {
      key: "crossfadeSkipAlbums",
      section: "Playback",
      title: "Never crossfade within an album",
      description: "Songs from one album played in order run straight into each other.",
      more: "Songs from the same album played in album order run straight into each other, with no gap and no fade.",
      control: (ids) => toggle(s, "crossfadeSkipAlbums", ids),
    },
    {
      key: "automix",
      section: "Playback",
      title: "Blend songs like a DJ (Automix)",
      description: "On mixes, radio and playlists with Mix on. Songs meet on the beat when they can.",
      more: "DJ-style transitions on playlists you turn Mix on for, mixes and radio. When two songs have a steady beat at close tempos they meet on the beat, otherwise they fade into each other. Platter works out each song's tempo and key on this computer while it's idle. Never on albums.",
      control: (ids) => toggle(s, "automix", ids),
    },
    {
      key: "automixWhere",
      section: "Playback",
      title: "Where Automix plays",
      description: "Playlists and mixes, or everywhere except albums.",
      more: "Playlists and mixes covers playlists with Mix on, mixes, radio and what plays after a list ends. Everywhere except albums adds songs, artists and Liked Songs.",
      note: s && !s.automix ? "Turn on Automix to use this." : undefined,
      control: (ids) => select(s, (x) => x.automixWhere, AUTOMIX_WHERE, ids, (v) => updateSettings({ automixWhere: v })),
    },
    {
      key: "automixStyle",
      section: "Playback",
      title: "Transition style",
      description: "Smooth blends for longer. Energetic keeps blends short and swaps songs on the bar.",
      note: s && !s.automix ? "Turn on Automix to use this." : undefined,
      control: (ids) => select(s, (x) => x.automixStyle, AUTOMIX_STYLE, ids, (v) => updateSettings({ automixStyle: v })),
    },
    {
      key: "flipPause",
      section: "Playback",
      title: "Pause to flip at the end of a side",
      description: "With the record player showing, playback stops between sides until you flip.",
      more: "When the record player shows in expanded now playing and an album plays from one side into the next, playback pauses at the end of the side, the tonearm lifts, and a Flip button plays on. Needs Show vinyl sides.",
      note: s && s.showVinylSides === false ? "Turn on Show vinyl sides to use this." : undefined,
      control: (ids) => toggle(s, "flipPause", ids),
    },
    // Loudness
    {
      key: "leveling",
      section: "Sound",
      title: "Level loudness between songs",
      description: "Plays songs at a similar volume. Smart keeps an album's own dynamics.",
      more: "Plays songs at a similar volume using your server's loudness analysis. Smart keeps an album's own dynamics while you play it in order and levels each song otherwise.",
      control: (ids) => select(s, (x) => x.leveling, LEVELING, ids, (v) => updateSettings({ leveling: v })),
    },
    {
      key: "preampDb",
      section: "Sound",
      title: "Volume before the equalizer",
      description: "Raises or lowers every song, from -12 to +12 dB. This device only.",
      more: "Raises or lowers every song, from -12 to +12 dB. Songs your server hasn't analysed get only this. This device only.",
      control: (ids) => <PreampControl ids={ids} />,
    },
    {
      key: "limiter",
      section: "Sound",
      title: "Keep loud parts from distorting",
      description: "Gently holds back peaks so the preamp and EQ boosts never distort. This device only.",
      control: (ids) => toggle(s, "limiter", ids),
    },
    // Equalizer
    {
      key: "eqEnabled",
      section: "Sound",
      title: "Use the equalizer",
      description: "Ten bands, presets, and a profile for each output. This device only.",
      more: "Shape the sound with ten bands from 31 Hz to 16 kHz. Drag the points, or focus one and use the arrow keys (Shift for bigger steps). Pick a preset, save your own, and give an output device its own profile. This device only.",
      control: (ids) => toggle(s, "eqEnabled", ids),
      block: () => <EqEditor />,
    },
    ...hifiRows(),
    // Sharing and friends
    {
      key: "unreleasedSharing",
      section: "Sharing and friends",
      title: "Share unreleased music",
      description: "Whether demos, leaks and other unreleased songs can leave this computer.",
      more: "Never keeps them off Discord, scrobbles, mixes, shared playlists, Blends, listening sessions and public lyrics. Always lets them through. Ask each time asks where Platter can, like before sharing a library or a transcription, and otherwise uses each feature's own choice under More. Picking a rule sets those choices; change one there to make an exception.",
      note: (() => {
        const n = s ? exceptionsTo(s).length : 0;
        return n ? `${n} ${n === 1 ? "feature is" : "features are"} set differently under More.` : undefined;
      })(),
      control: (ids) =>
        select(s, (x) => x.unreleasedSharing ?? "ask", UNRELEASED_RULE_OPTIONS, ids, chooseRule),
    },
    {
      key: "sharePublicLinks",
      section: "Sharing and friends",
      title: "Public links for released music",
      description: "Share menus can copy a song.link page that opens in any music service.",
      more: "Share menus can copy a song.link page that opens the song or album in any music service. Platter finds it with iTunes Search, sending only the artist, title and album. Unreleased music never gets one. Off leaves only Platter links.",
      control: (ids) => toggle(s, "sharePublicLinks", ids),
    },
    {
      key: "shareCardQr",
      section: "Sharing and friends",
      title: "Share cards include a QR code",
      description: "Share card images get a QR code that opens the item in Platter.",
      more: "Share card images get a QR code that opens the song, album or playlist in Platter for people on your Plex server. It holds only the Platter link, never your server's address. You can turn it off on each card too.",
      control: (ids) => toggle(s, "shareCardQr", ids),
    },
    {
      key: "shareCardSongLink",
      section: "Sharing and friends",
      title: "Include the song.link line",
      description: "Share cards for released music show their song.link page.",
      more: "Share cards for released music show their song.link page. Unreleased music never does. Needs Public links for released music.",
      note: s && !s.sharePublicLinks ? "Turn on Public links for released music to use this." : undefined,
      control: (ids) => toggle(s, "shareCardSongLink", ids),
    },
    // Sharing: your music through Plex
    {
      key: "shareMyMusic",
      section: "Sharing and friends",
      group: "Share my music",
      title: "Share my music",
      description: "Invite someone to your Plex server and pick which music libraries.",
      more: "Invite someone to your Plex server by Plex username or email, or send a link or QR code their Platter opens. You pick which music libraries; your other libraries are never shared. Platter tells you how many unreleased albums they hold first.",
      control: (ids) => <ShareMyMusic ids={ids} />,
    },
    {
      key: "sharePeople",
      section: "Sharing and friends",
      group: "Share my music",
      title: "People with access",
      description: "Everyone your server is shared with, what they can see and when they were last on it.",
      control: () => null,
      block: () => <PeopleWithAccess />,
    },
    {
      key: "shareDefaultLibraries",
      section: "Sharing and friends",
      group: "Share my music",
      title: "Libraries I share by default",
      description: "What a new share starts with. Music libraries only.",
      more: "What a share starts with, and what Let them listen from my server shares in one click. Music libraries only. None until you pick.",
      control: () => null,
      block: () => <DefaultLibraries />,
    },
    {
      key: "shareHideUnreleased",
      section: "Sharing and friends",
      group: "Share my music",
      title: "Hide my unreleased music from people I share with",
      description: "Each share leaves your unreleased albums out. Needs Plex Pass.",
      more: "Platter labels your unreleased albums in Plex and each share leaves that label out, so friends never see them. This uses Plex's own sharing restrictions, which need Plex Pass. Without it, Platter asks before sharing a library with unreleased music.",
      control: (ids) => toggle(s, "shareHideUnreleased", ids),
    },
    {
      key: "shareAllowDownloads",
      section: "Sharing and friends",
      group: "Share my music",
      title: "Allow downloads for people I share with",
      description: "Lets them download your music for offline listening in their Plex apps.",
      more: "Lets them download your music for offline listening in their Plex apps. Plex's own rules decide where and how they can play it.",
      control: (ids) => toggle(s, "shareAllowDownloads", ids),
    },
    {
      key: "requestWeeklyLimit",
      section: "Sharing and friends",
      group: "Share my music",
      title: "Albums a week for Straight away",
      description: "Past this, their requests wait for your yes.",
      more: "Someone set to Straight away gets this many albums a week without asking. After that their requests wait on What's New for your yes, like everyone else's.",
      shown: s?.requestsFromOthers !== false,
      control: (ids) => select(s, (x) => String(x.requestWeeklyLimit ?? 20), REQUEST_LIMIT_OPTIONS, ids, (v) => updateSettings({ requestWeeklyLimit: Number(v) })),
    },
    {
      key: "requestGroupNotices",
      section: "Sharing and friends",
      group: "Share my music",
      title: "Group request notifications",
      description: "A few requests from one person in a row notify you once.",
      more: "When someone requests a few albums within a few minutes, you get one notification for the first. They all wait on What's New. On this device only.",
      shown: s?.requestsFromOthers !== false,
      control: (ids) => toggle(s, "requestGroupNotices", ids),
    },
    {
      key: "shareAcceptInvites",
      section: "Sharing and friends",
      group: "Invites to you",
      title: "Accept invites inside Platter",
      description: "A notification and a card when a friend shares their server.",
      more: "When a friend shares their server with you, a notification and a card show here and in Listen along. One click accepts it and adds their music; no visit to plex.tv needed. In a listening session, only your Plex username goes to the host, sealed, when they ask.",
      control: (ids) => toggle(s, "shareAcceptInvites", ids),
    },
    {
      key: "shareInvites",
      section: "Sharing and friends",
      group: "Invites to you",
      title: "Invites waiting",
      description: "Servers friends shared with you that you haven't accepted yet.",
      control: () => null,
      block: () => <InvitesToYou />,
    },
    // Friends, following and listening sessions
    {
      key: "showFriendActivity",
      section: "Sharing and friends",
      cap: "friendActivity",
      title: "Show friend activity",
      description: "What others on your Plex server played this week. Owners only.",
      more: "The people button in the top bar shows what others on your Plex server listened to in the last week. Only the server's owner can see it, because Plex shares sessions and history with the owner only.",
      control: (ids) => toggle(s, "showFriendActivity", ids),
    },
    {
      key: "shareActivity",
      section: "Sharing and friends",
      title: "Share my activity with Platter friends",
      description: "Lets friends see what you're playing right now.",
      more: "Lets your server show what you're playing right now. Off, and in a private session, Platter stops telling your server about the song as it plays, so friends don't see you live. Songs you finish still go into your server's history, which its owner and other apps can see.",
      control: (ids) => toggle(s, "shareActivity", ids),
    },
    {
      key: "releaseRadar",
      section: "Library",
      title: "Find new releases (release radar)",
      description: "New albums from artists you follow and play most, as a Home shelf.",
      more: "New albums, EPs and singles from artists you follow and play most, looked up on MusicBrainz about once a day, as a shelf on Home and a page. Only their names and MusicBrainz ids are sent.",
      control: (ids) => toggle(s, "releaseRadar", ids),
    },
    {
      key: "fansAlsoLikeCatalogue",
      section: "Library",
      title: "Show artists you don't have in Fans also like",
      description: "Artist pages add similar artists you don't have, dimmed.",
      more: "Artist pages add similar artists that aren't in your library (from ListenBrainz), dimmed, opening their own page.",
      control: (ids) => toggle(s, "fansAlsoLikeCatalogue", ids),
    },
    {
      key: "homeNewToYou",
      section: "Library",
      title: "New to you on Home",
      description: "A Home shelf of artists like your favourites that you don't have yet.",
      more: "A shelf of artists like the ones you play most that aren't in your library, from ListenBrainz and MusicBrainz. Only artists' MusicBrainz ids are sent.",
      control: (ids) => toggle(s, "homeNewToYou", ids),
    },
    {
      key: "dismissedArtists",
      section: "Library",
      page: "hidden",
      title: "Artists you're not interested in",
      description: "Never suggested in New to you or Fans also like. Undo one to see them again.",
      more: "They never show up in New to you, Fans also like or on artist pages' suggestions. Undo one to see them again.",
      control: () => null,
      block: () => <DismissedArtists />,
    },
    {
      key: "listenGuestsControl",
      section: "Sharing and friends",
      group: "Listening sessions",
      title: "Let others change what's playing",
      description: "Guests can play, pause, skip and reorder. Off, they can only add songs.",
      more: "People listening along with you can play, pause, skip and reorder the queue. Off, they can only add songs. Each person plays from your Plex library when they have access to it, else from their own copy of the same song.",
      control: (ids) => toggle(s, "listenGuestsControl", ids),
    },
    {
      key: "listenInviteHours",
      section: "Sharing and friends",
      group: "Listening sessions",
      title: "Invites expire after",
      description: "Each invite link works for one person, once, and stops working after this long.",
      control: (ids) => select(s, (x) => String(x.listenInviteHours), INVITE_HOURS, ids, (v) => updateSettings({ listenInviteHours: Number(v) })),
    },
    {
      key: "listenName",
      section: "Sharing and friends",
      group: "Listening sessions",
      title: "Your name in listening sessions",
      description: "What others in a session see. Empty uses your Plex name.",
      more: "What others in a session see, next to the songs you add. Leave it empty to use your Plex name. Nobody sees your email, location or computer.",
      control: (ids) => <ListenTextSetting field="listenName" placeholder="Your Plex name" maxLength={40} ids={ids} />,
    },
    {
      key: "listenRelay",
      section: "Sharing and friends",
      group: "Listening sessions",
      title: "Relay for listening sessions",
      description: "Everyone connects only to this relay, sealed end to end. Empty uses ntfy.sh.",
      more: "Everyone in a session connects only to this relay, never to each other, so nobody learns anyone else's address. Messages are sealed end to end; the relay sees only scrambled text and each person's connection. Leave it empty for ntfy.sh (free, about 250 messages a day per connection), or enter your own ntfy server. Your own server's address goes in your invites.",
      control: (ids) => <ListenTextSetting field="listenRelay" placeholder="https://ntfy.sh" maxLength={200} ids={ids} />,
    },
    {
      key: "listenHideUnreleased",
      section: "Sharing and friends",
      group: "Unreleased music, feature by feature",
      title: "Hide unreleased song titles from guests",
      description: "Guests without access see a song isn't in their library, but not its title.",
      more: "Unreleased songs only play for people with access to your Plex library. Everyone else sees that a song isn't in their library; with this on, they don't see its title or artist either.",
      control: (ids) => toggle(s, "listenHideUnreleased", ids),
    },
    {
      key: "collabPlaylists",
      section: "Sharing and friends",
      title: "Edit playlists with friends",
      description: "Invite people to your playlists and join theirs.",
      more: "Invite people to your playlists and join theirs. Nothing happens until someone sends or opens an invite. Everyone plays from the servers they already reach; only song details travel, sealed, through the relay.",
      control: (ids) => toggle(s, "collabPlaylists", ids),
    },
    {
      key: "blends",
      section: "Sharing and friends",
      title: "Make Blends with friends",
      description: "Playlists made from your tastes and friends', new each day.",
      more: "Playlists made from your tastes and friends', new each day. Only a short summary of what you play most and your recent likes is shared, never plays from a private session.",
      control: (ids) => toggle(s, "blends", ids),
    },
    {
      key: "shareUnreleased",
      section: "Sharing and friends",
      group: "Unreleased music, feature by feature",
      title: "Include unreleased music in what I share",
      description: "Shared playlists, your taste summary and Blends.",
      more: "Off, demos and other unreleased songs stay out of your shared playlists, your taste summary and your Blends.",
      control: (ids) => toggle(s, "shareUnreleased", ids),
    },
    {
      key: "shareOfferServer",
      section: "Sharing and friends",
      group: "Listening sessions",
      title: "Offer \"Let them listen from my server\"",
      description: "Invite a guest who can't hear a song to your server in one click.",
      more: "In a session you host, when someone can't hear a song because it isn't on any server they can reach, you can invite them to your Plex server with one click. Only the libraries you share by default, never the others.",
      control: (ids) => toggle(s, "shareOfferServer", ids),
    },
    // Integrations: scrobbling
    {
      key: "scrobbleLastfm",
      section: "Integrations",
      page: "scrobbling",
      title: "Send plays to Last.fm",
      description: "Sends songs you play past halfway to your Last.fm profile.",
      more: "Sends each song you play for more than half its length (or 4 minutes) to your Last.fm profile, and what's playing now. Uses your Last.fm API key and its shared secret, and you sign in on Last.fm's site. Not during a private session.",
      note: rowNote(lastfmNote(scrobble)),
      control: (ids) => <LastfmControl ids={ids} />,
    },
    {
      key: "scrobbleListenbrainz",
      section: "Integrations",
      page: "scrobbling",
      title: "Send plays to ListenBrainz",
      description: "Sends the same plays to ListenBrainz with your user token.",
      more: "Sends the same plays to ListenBrainz with your user token. Not during a private session. Plays made offline wait and go out later.",
      note: rowNote(listenbrainzNote(scrobble)),
      control: (ids) => <ListenBrainzControl ids={ids} />,
    },
    {
      key: "scrobbleUnreleased",
      section: "Sharing and friends",
      group: "Unreleased music, feature by feature",
      title: "Scrobble unreleased music",
      description: "Off keeps plays of unreleased music on this computer.",
      more: "Demos and other unreleased versions go to your scrobbling profiles too. Off keeps them on this computer.",
      control: (ids) => toggle(s, "scrobbleUnreleased", ids),
    },
    // Integrations: AI provider
    {
      key: "aiProvider",
      section: "Integrations",
      page: "ai",
      title: "AI provider",
      description: "Turns a description into a prompted playlist. None hides them.",
      more: "Turns a description into a prompted playlist's rules and picks songs for it. None keeps prompted playlists hidden. The provider gets your description, your library's genre, style and mood names, and the titles of songs that match. Unreleased music is never sent.",
      control: (ids) => select(s, (x) => x.aiProvider, AI_PROVIDERS, ids, (v) => updateSettings({ aiProvider: v }).then(() => void loadAi())),
    },
    ...(s?.aiProvider === "anthropic"
      ? ([
          {
            key: "aiKey",
            section: "Integrations",
            page: "ai",
            title: "Anthropic key",
            description: "Your own key from the Anthropic Console, kept in this computer's keychain.",
            note: ai?.hasKey ? "Saved." : undefined,
            control: (ids) => <AiKeyControl ids={ids} />,
          },
          {
            key: "aiModel",
            section: "Integrations",
            page: "ai",
            title: "AI model",
            description: "The Claude model prompted playlists use.",
            control: (ids) => <AiModelField ids={ids} />,
          },
        ] satisfies RowDef[])
      : []),
    // Integrations: Tautulli (its play history counts in Stats once set up)
    {
      key: "tautulliUrl",
      section: "Integrations",
      page: "tautulli",
      title: "Tautulli address",
      description: "Tautulli's address with its port. Empty keeps Tautulli off.",
      more: "Tautulli's address with its port, like http://192.168.x.x:8181. With its API key saved, your music plays in Tautulli count in Stats, mixes and recaps. Empty keeps Tautulli off.",
      control: (ids) => <TautulliUrlField ids={ids} />,
    },
    {
      key: "tautulliKey",
      section: "Integrations",
      page: "tautulli",
      title: "Tautulli key",
      description: "From Tautulli's Settings > Web Interface, kept in this computer's keychain.",
      more: "From Tautulli's Settings > Web Interface. Kept in this computer's keychain, never in settings or logs. Platter only reads your history.",
      note: history.tautulli?.hasKey ? "Saved." : undefined,
      control: (ids) => <TautulliKeyControl history={history} ids={ids} />,
    },
    // Integrations: Lidarr (only its address and key fields until both are set)
    ...(lidarr.ready
      ? []
      : ([
          {
            key: "lidarrGuide",
            section: "Integrations",
            page: "lidarr",
            title: "Set up Lidarr step by step",
            description: "The address, the key, the safety check and what the artist modes mean.",
            more: "A short guide through the address, the key and the safety check, and what Managed and Protected artists and the staging folder mean.",
            control: () => <GuideButton id="lidarr" />,
          },
        ] satisfies RowDef[])),
    {
      key: "bringLikeArrivals",
      section: "Integrations",
      page: "libraryImport",
      group: "Bringing music over",
      title: "Like songs as they arrive",
      description: "When an album Lidarr got arrives, its songs you liked elsewhere are liked here.",
      more: "Songs you liked in the other app that weren't in your library are liked once their album arrives. Each song only once: unlike one and it stays unliked.",
      control: (ids) => toggle(s, "bringLikeArrivals", ids),
    },
    {
      key: "bringFillPlaylists",
      section: "Integrations",
      page: "libraryImport",
      group: "Bringing music over",
      title: "Add missing songs to playlists as they arrive",
      description: "A brought-over playlist's greyed songs are added once their album arrives.",
      more: "Playlists you brought over show the songs your library lacks greyed. Once their album arrives they're added at the end. Each song only once: take one out and it stays out.",
      control: (ids) => toggle(s, "bringFillPlaylists", ids),
    },
    {
      key: "bringInPerDay",
      section: "Integrations",
      page: "libraryImport",
      group: "Bringing music over",
      title: "Albums a day",
      description: "How many albums Lidarr is asked for each day, most loved first.",
      more: "Albums go to Lidarr a few a day so it and its indexers aren't flooded. The same pace as Bring in my listening.",
      control: (ids) => <PerDaySelect ids={ids} />,
    },
    {
      key: "lidarrUrl",
      section: "Integrations",
      page: "lidarr",
      title: "Lidarr address at home",
      description: "Lidarr's address on your home network, with its port. Empty keeps it off.",
      more: "Lidarr's address on your home network, with its port and any extra path, like http://192.168.x.x:8686. Platter tries it first. Empty keeps Lidarr off.",
      control: (ids) => <LidarrUrlField field="lidarrUrl" placeholder="http://192.168.x.x:8686" ids={ids} />,
    },
    {
      key: "lidarrAwayUrl",
      section: "Integrations",
      page: "lidarr",
      title: "Lidarr address away from home",
      description: "Optional. How Lidarr is reached away from home. Use https.",
      more: "Optional. How Lidarr is reached away from home, through a reverse proxy, a tunnel or a VPN you set up. Tried when the home address doesn't answer. Use https so the key travels encrypted.",
      control: (ids) => <LidarrUrlField field="lidarrAwayUrl" placeholder="https://lidarr.example.com" ids={ids} />,
    },
    {
      key: "lidarrKey",
      section: "Integrations",
      page: "lidarr",
      title: "Lidarr key",
      description: "From Lidarr's Settings > General. Kept in this computer's keychain, never in settings or logs.",
      note: lidarr.status?.hasKey ? "Saved." : undefined,
      control: (ids) => <LidarrKeyControl lidarr={lidarr} ids={ids} />,
    },
    ...(lidarr.status?.urlSet || lidarr.status?.awaySet
      ? ([
          {
            key: "lidarrStagingFolder",
            section: "Integrations",
            page: "lidarr",
            title: "Lidarr staging folder",
            description: "A Lidarr root folder no Plex library scans, for protected artists.",
            more: "A Lidarr root folder, as Lidarr sees it, that no Plex library scans. Lidarr only ever downloads protected artists here, and better copies wait here until they're swapped in. The setup check makes sure Plex can't see it.",
            control: (ids) => <StagingFolderField ids={ids} />,
          },
        ] satisfies RowDef[])
      : []),
    ...(lidarr.status?.awaySet
      ? ([
          {
            key: "lidarrHeaders",
            section: "Integrations",
            page: "lidarr",
            title: "Extra sign-in details for the away address",
            description: "Optional. Headers your proxy needs, sent only to the away address.",
            more: "Optional. Headers your proxy needs, one per line as Name: value (for example a Cloudflare Access service token). Sent only to the away address and kept in this computer's keychain.",
            note: lidarr.status.headerNames.length ? `Saved: ${lidarr.status.headerNames.join(", ")}.` : undefined,
            control: (ids) => <LidarrHeadersControl lidarr={lidarr} ids={ids} />,
          },
        ] satisfies RowDef[])
      : []),
    // Integrations: Lidarr away from home. A line only when it can't be reached.
    {
      key: "lidarrAway",
      section: "Integrations",
      page: "lidarr",
      title: "Away from home",
      description: "Lidarr can't be reached away from home right now.",
      more: "Away from home, Platter tries Lidarr's home address, then its away address, then a computer of yours at home that has Lidarr set up, then the Bridge. Your devices find each other through your Plex account, so there's nothing to set up.",
      control: () => null,
      block: () => <LidarrAwayProblem sides={homeSides} />,
      shown: !!homeSides?.problem,
    },
    {
      key: "homeSideLidarr",
      section: "Integrations",
      page: "lidarr",
      title: "Let my other devices use Lidarr through this computer",
      description: "Your other devices reach Lidarr through this one while it's open. This device only.",
      more: "With Lidarr set up here, your phone and your other devices on the same Plex account can get albums, see what's missing and send requests through this computer while Platter is open, at home or through the sealed relay away from home. Every request is checked the way the Bridge checks it, and the key never leaves this computer. This device only.",
      control: (ids) => toggle(s, "homeSideLidarr", ids),
    },
    // Integrations: Platter Bridge. Each benefit's switch, under the card's More once one is paired.
    {
      key: "bridgeLyrics",
      section: "Integrations",
      page: "bridge",
      shown: !!bridge?.bridges.length,
      title: "Lyrics for every song",
      description: "The Bridge transcribes songs without lyrics at night, for every device.",
      more: "While it does, this computer leaves its own background transcribing to the Bridge.",
      control: (ids) => toggle(s, "bridgeLyrics", ids),
    },
    {
      key: "bridgeLyricsHours",
      section: "Integrations",
      page: "bridge",
      shown: !!bridge?.bridges.length,
      title: "Night hours",
      description: "When the Bridge transcribes, in this computer's time zone.",
      more: "The Bridge starts songs only in these hours, and waits while other programs keep your server busy. Any time means whenever the server is calm.",
      control: (ids) => select(s, (x) => x.bridgeLyricsHours ?? "night", NIGHT_HOURS_OPTIONS, ids, (v) => void updateSettings({ bridgeLyricsHours: v })),
    },
    {
      key: "bridgeLidarr",
      section: "Integrations",
      page: "bridge",
      shown: !!bridge?.bridges.length,
      title: "Lidarr from anywhere",
      description: "The Bridge does Lidarr work and answers your devices away from home.",
      more: "The Bridge catches up with Lidarr after downloads, keeps protected albums safe, runs Bring in my listening and lets your phone reach Lidarr away from home, even with every computer off. Off, it leaves all of that to your computers.",
      control: (ids) => toggle(s, "bridgeLidarr", ids),
    },
    ...(lidarr.ready
      ? ([
          {
            key: "lidarrCheck",
            section: "Integrations",
            page: "lidarr",
            cap: "realFilePaths",
            title: "Check Lidarr's setup",
            description: "Makes sure Lidarr can never touch your existing library. Only reads.",
            more: "Reads Lidarr's folders and settings and compares them with your Plex music, by contents as well as paths, to make sure Lidarr can never touch your existing library. Only reads; nothing in Lidarr changes.",
            control: () => <LidarrCheckButton lidarr={lidarr} />,
            block: () => <LidarrFindings lidarr={lidarr} />,
          },
          {
            key: "lidarrWork",
            section: "Integrations",
            page: "lidarr",
            title: "Practice mode and catching up",
            description: "Catches up with what Lidarr did. The first week it only notes what it would do.",
            more: "Platter catches up with what Lidarr did while it was closed, on start and every few minutes. For the first week after setup it only writes down what it would add or swap.",
            control: () => null,
            block: () => <LidarrWorkStatus />,
          },
          {
            key: "lidarrArtists",
            section: "Integrations",
            page: "lidarr",
            title: "Managed and protected artists",
            description: "Lidarr owns Managed artists' folders; Protected ones stay curated.",
            more: "Managed: Lidarr owns the artist's folder and imports, renames and upgrades there. Protected: curated files; Lidarr only downloads into the staging folder and Platter adds albums to your library itself. Picked for each artist from its files' tags, album types and Lidarr's exclusions.",
            control: () => null,
            block: () => <LidarrArtistsList />,
          },
          {
            key: "lidarrWaiting",
            section: "Integrations",
            page: "lidarr",
            title: "Waiting for you",
            description: "Protected albums Lidarr brought into the staging folder.",
            more: "Protected albums Lidarr brought into the staging folder. Complete official albums can be added with the names and tags the artist's other albums use; anything else says why it waits.",
            control: () => null,
            block: () => <LidarrWaitingList />,
          },
          {
            key: "lidarrScanAfterImport",
            section: "Integrations",
            page: "lidarr",
            title: "Ask Plex to scan after Lidarr imports",
            description: "Asks Plex to scan a Managed album's folder when it doesn't show up.",
            more: "When a Managed album doesn't show up in Plex within about two minutes, Platter asks Plex to scan just its folder.",
            control: (ids) => toggle(s, "lidarrScanAfterImport", ids),
          },
          {
            key: "lidarrFreeStaging",
            section: "Integrations",
            page: "lidarr",
            title: "Free up staging space after adding",
            description: "Frees a protected album's staging copy once it's in your library.",
            more: "Once a protected album is in your library and Lidarr has stopped looking for it, its copy in the staging folder is freed (into Lidarr's recycling bin when it has one).",
            control: (ids) => toggle(s, "lidarrFreeStaging", ids),
          },
          {
            key: "lidarrTapToGet",
            section: "Integrations",
            page: "lidarr",
            title: "Tap a song you don't have to get and play it",
            description: "A search result you don't own plays on tap. Lidarr gets just its album.",
            more: "Songs you don't have show Get in search. A tap opens Now playing on the song while Lidarr adds just that album, without the rest of the artist, and grabs one copy. It plays as soon as it's here. Off makes a tap open the album's page instead.",
            note: <LastGetLine />,
            control: (ids) => toggle(s, "lidarrTapToGet", ids),
          },
          {
            key: "lidarrPreferFastest",
            section: "Integrations",
            page: "lidarr",
            title: "Prefer the fastest download",
            description: "For a tap, usenet comes first, then the best seeded torrent, at your quality or better.",
            more: "One search is run for the album. Of the copies Lidarr approves, Platter picks the one likely to arrive first: usenet before torrents, then the torrent with the most seeders, keeping to your quality profile's cutoff when a copy meets it. Off takes Lidarr's own first choice.",
            shown: s?.lidarrTapToGet !== false,
            control: (ids) => toggle(s, "lidarrPreferFastest", ids),
          },
          {
            key: "lidarrPlayArriving",
            section: "Integrations",
            page: "lidarr",
            cap: "realFilePaths",
            title: "Play while it's still arriving",
            description: "Plays the finished download before Lidarr and Plex take it in, then moves to Plex's copy.",
            more: "Once the download is finished, the song plays straight from it on this computer, which needs Where this computer reaches your music. When Plex has its own copy, playback moves over at the same spot. Off waits for Plex.",
            shown: s?.lidarrTapToGet !== false,
            control: (ids) => toggle(s, "lidarrPlayArriving", ids),
          },
          {
            key: "requestsFromOthers",
            section: "Integrations",
            page: "lidarr",
            title: "Let people you share with request music",
            description: "They see Request where you see Get. You say yes or no.",
            more: "Someone your server is shared with sees Request on music it doesn't have. You get a notification and it waits on What's New for your yes, which sends it through Get with every check. Choose per person in Sharing and friends, People with access. Off hides Request for everyone.",
            control: (ids) => toggle(s, "requestsFromOthers", ids),
          },
          {
            key: "lidarrChanges",
            section: "Integrations",
            page: "lidarr",
            title: "Changes",
            description: "Everything Platter changed in Lidarr, when and from which device, with Put it back where it can.",
            control: () => null,
            block: () => <LidarrChangesList />,
          },
          {
            key: "showMissingReleases",
            section: "Integrations",
            page: "lidarr",
            title: "Show missing official releases",
            description: "Artist pages list official albums you don't have, with Request.",
            more: "Artist pages list official releases you don't have in their discography, with Get, of the kinds set in Album kinds for Get albums. Only while Lidarr's setup check passes. Anything sharing a title with an album in your library, unreleased ones included, is left out.",
            control: (ids) => toggle(s, "showMissingReleases", ids),
          },
          {
            key: "autoRequest",
            section: "Integrations",
            page: "lidarr",
            title: "Auto-request new albums",
            description: "New official albums from artists you pick are requested on their own. This device only.",
            more: "For artists you turn it on for (from the artist page's menu), new official albums the release radar finds are requested on their own, after the same setup check as any request. Bootlegs, promos and anything sharing a title with an album in your library are never requested. This device only.",
            control: (ids) => toggle(s, "autoRequest", ids),
          },
          {
            key: "autoRequestCap",
            section: "Integrations",
            page: "lidarr",
            title: "Most automatic requests a day",
            description: "Albums past the limit wait for the next day. A summary notification says what was requested.",
            control: (ids) => select(s, (x) => String(x.autoRequestCap ?? 3), AUTO_CAP_OPTIONS, ids, (v) => updateSettings({ autoRequestCap: Number(v) })),
          },
          {
            key: "autoRequestArtists",
            section: "Integrations",
            page: "lidarr",
            title: "Artists with auto-request",
            description: "Turning one off stops new requests for that artist. Requests already sent stay in Lidarr.",
            control: () => null,
            block: () => <AutoArtistsList />,
          },
          ...(s?.lidarrStagingFolder
            ? ([
                {
                  key: "upgradesAsk",
                  section: "Integrations",
                  page: "lidarr",
                  title: "Upgrades: ask before each swap",
                  description: "On shows what would change and waits for you. Off swaps on its own.",
                  more: "On shows what would change and waits for you. Off swaps a better copy in on its own once it arrives, under the same rules. An album folder with anything kept as it is always asks, song by song.",
                  control: (ids) => toggle(s, "upgradesAsk", ids),
                },
                {
                  key: "upgradeOfficialPrefix",
                  section: "Integrations",
                  page: "lidarr",
                  title: "Official source note starts with",
                  description: "A song without an ISRC swaps only when its source note starts with this.",
                  more: "A song without an ISRC only swaps when a tag says it's the released recording, its source note starts with this, it's on the confirmed release, the lengths match and it sounds the same as the new copy.",
                  control: (ids) => <OfficialPrefixField ids={ids} />,
                },
                {
                  key: "musicShare",
                  section: "Integrations",
                  page: "lidarr",
                  cap: "realFilePaths",
                  title: "Where this computer reaches your music",
                  description: "Your music share and the staging folder, as this computer sees them. This device only.",
                  more: "Swaps run on this computer, on the files themselves, so Platter needs your music share and the staging folder as this computer sees them. Nothing is written without this. Replaced files go to a _to_delete folder Plex is told to ignore, or a folder you pick outside your music, never deleted. This device only.",
                  control: () => null,
                  block: () => <ShareMapEditor />,
                },
                {
                  key: "lidarrUpgrades",
                  section: "Integrations",
                  page: "lidarr",
                  title: "Upgrades",
                  description: "Better copies asked for from this computer, and the swaps made. Undo is on each album's page.",
                  control: () => null,
                  block: () => <UpgradesList />,
                },
              ] satisfies RowDef[])
            : []),
          {
            key: "lidarrRequests",
            section: "Integrations",
            page: "lidarr",
            title: "Requests",
            description: "Albums requested from this computer and where each stands in Lidarr.",
            more: "Albums requested from this computer and where each stands in Lidarr. Waiting ones send when Platter can reach Lidarr, after the setup check runs again.",
            control: () => null,
            block: () => <LidarrRequestsList />,
          },
        ] satisfies RowDef[])
      : []),
    // Integrations: Discord
    {
      key: "discordStatus",
      section: "Integrations",
      page: "discord",
      title: "Show what you're playing on Discord",
      description: "Shows the song, artist and cover while music plays. This device only.",
      more: "Your Discord profile shows the song, artist, album and cover while music plays. Needs the Discord app open on this computer. A private session hides it. This device only.",
      note: discordNote(discord, s?.discordStatus),
      control: (ids) => (discordOff ? null : toggle(s, "discordStatus", ids)),
    },
    ...(discordOff
      ? []
      : ([
          {
            key: "discordClearOnPause",
            section: "Integrations",
            page: "discord",
            title: "Clear status when paused",
            description: "Pausing removes the status. Off keeps the song on your profile, marked as paused.",
            shown: discordOn,
            control: (ids) => toggle(s, "discordClearOnPause", ids),
          },
          {
            key: "discordShowUnreleased",
            section: "Sharing and friends",
            group: "Unreleased music, feature by feature",
            title: "Show unreleased music on Discord",
            description: "Off shows nothing on Discord while an unreleased song plays. This device only.",
            control: (ids) => toggle(s, "discordShowUnreleased", ids),
          },
          {
            key: "discordUnreleasedArt",
            section: "Sharing and friends",
            group: "Unreleased music, feature by feature",
            title: "Show unreleased cover art on Discord",
            description: "Off shows the Platter icon instead, so the art is never uploaded. This device only.",
            more: "Off shows the Platter icon instead of the cover for unreleased songs, so their art is never looked up or uploaded.",
            control: (ids) => toggle(s, "discordUnreleasedArt", ids),
          },
          {
            key: "discordFindSong",
            section: "Integrations",
            page: "discord",
            title: "Find this song button",
            description: "Adds a button that opens the song's song.link page, for released songs only.",
            note: s && !s.sharePublicLinks ? "Turn on Public links for released music to use this." : undefined,
            shown: discordOn,
            control: (ids) => toggle(s, "discordFindSong", ids),
          },
          {
            key: "discordCoverSource",
            section: "Integrations",
            page: "discord",
            title: "Cover art source",
            description: "Where the cover on your Discord status comes from.",
            more: "Cover Art Archive finds a public copy of the album's cover by its MusicBrainz id, or for released albums by an exact artist and title match. Covers it doesn't have, mostly for unreleased songs, can be uploaded to Imgur (with your own client ID, used first) or to a free image host that needs no account. Anyone with the link can see an uploaded cover, and Platter can't delete it. Automatic tries Litterbox first, then x0.at, uguu.se and catbox.moe; Litterbox keeps a cover 3 days, x0.at up to 100 days, uguu.se 3 hours, and catbox.moe keeps it. Platter uploads a cover again before its host deletes it. Without a cover, Discord shows the Platter icon.",
            note: errorNote(coversNote(discord, s?.discordUploadCovers)),
            shown: discordOn,
            control: (ids) =>
              select(s, coverSourceOf, COVER_SOURCES, ids, (v) => void updateSettings(coverSourcePatch(v))),
            block: () =>
              s && (s.discordUploadCovers || s.discordUploadArt) ? (
                <div className="setting-sub">
                  <CoverHostRow s={s} />
                  <ImgurRow status={discord} setKey={setImgurKey} />
                </div>
              ) : null,
          },
          {
            key: "discordStatusLine",
            section: "Integrations",
            page: "discord",
            title: "Discord status shows",
            description: "What the member list shows after \"Listening to\": Platter, the artist, or the song.",
            shown: discordOn,
            control: (ids) =>
              select(s, (x) => x.discordStatusLine, DISCORD_STATUS_LINE, ids, (v) => updateSettings({ discordStatusLine: v })),
          },
          {
            key: "discordListenAlong",
            section: "Integrations",
            page: "discord",
            title: "Show the Listen along button on Discord",
            description: "Shows a listening session you host and how many are in it.",
            more: "While you host a listening session, your Discord status shows it and how many are listening. Joining from Discord isn't there yet, so send the invite link.",
            shown: discordOn,
            control: (ids) => toggle(s, "discordListenAlong", ids),
          },
        ] satisfies RowDef[])),
    // Lyrics and the full-window view
    {
      key: "lyricsMode",
      section: "Lyrics",
      title: "Show synced lyrics automatically",
      description: "Opens the expanded view on lyrics while now playing is open.",
      more: "Auto-show opens the expanded view on lyrics when a song with synced lyrics starts while now playing is open. Off shows lyrics only from the lyrics button.",
      control: (ids) => select(s, (x) => x.lyricsMode, LYRICS_MODE, ids, (v) => updateSettings({ lyricsMode: v })),
    },
    {
      key: "lyricsLrclib",
      section: "Lyrics",
      title: "Lyrics from LRCLIB when your server has none",
      description: "Looks songs up on LRCLIB, a free lyrics library. Kept 30 days.",
      more: "Looks the song up on LRCLIB, a free lyrics library, sending its artist, title, album and length. Songs it doesn't know are searched by artist and title only. Results are kept for 30 days.",
      control: (ids) => toggle(s, "lyricsLrclib", ids),
    },
    {
      key: "lyricsPreferOwn",
      section: "Lyrics",
      title: "Prefer my own lyrics over your server's",
      description: "Songs you've written lyrics for show yours, even when your server has some.",
      more: "Songs you've written lyrics for in Platter show yours, even when your server has some. Off shows yours only when your server has none, unless you chose otherwise for the song.",
      control: (ids) => toggle(s, "lyricsPreferOwn", ids),
    },
    {
      key: "lyricsLiveTiming",
      section: "Lyrics",
      title: "Show borrowed timing on live versions",
      description: "Live versions follow the studio recording's timing anyway.",
      more: "Lyrics found by artist and title are usually timed to the studio recording, so on a live version they show without timing. On keeps that timing and follows along anyway. Lyrics files next to the song and your own lyrics always keep theirs.",
      control: (ids) => toggle(s, "lyricsLiveTiming", ids),
    },
    {
      key: "lyricsReleasedForVersions",
      section: "Lyrics",
      title: "Use released lyrics for other versions",
      description: "Other versions show the released version's lyrics without asking.",
      more: "Unreleased songs, alternate versions, live takes and songs with a version tag show lyrics found for the released version without asking. Off asks first with Use anyway, unless the check finds they match the song.",
      control: (ids) => toggle(s, "lyricsReleasedForVersions", ids),
    },
    {
      key: "lyricsChoices",
      section: "Lyrics",
      title: "Lyrics choices and timing",
      description: "Lyrics you picked, turned off or retimed. Reset lets Platter pick again.",
      more: "Lyrics you picked with Wrong lyrics?, songs you turned lyrics off for, and timing you moved. They win over everything automatic and travel in your sync file. Reset lets Platter pick again.",
      control: (ids) => <LyricsChoicesControl labelledBy={ids.title} />,
    },
    {
      key: "showPronunciation",
      section: "Lyrics",
      title: "Show pronunciation",
      description: "A romanised line under Japanese, Korean, Chinese, Hindi and Cyrillic lyrics.",
      more: "A romanised line under lyrics in Japanese, Korean, Chinese, Hindi and Cyrillic scripts, also in synced lyrics as they play. Made on this computer; nothing is sent anywhere.",
      control: (ids) => toggle(s, "showPronunciation", ids),
    },
    {
      key: "translationService",
      section: "Lyrics",
      title: "Translation service address",
      description: "A LibreTranslate-compatible service. Not set: lyrics are never sent.",
      more: "The address of a LibreTranslate-compatible service, like one you run yourself. With one set, lyrics have a Translate button that sends their lines there. Not set: lyrics are never sent for translation.",
      control: (ids) => <ListenTextSetting field="translationService" placeholder="https://translate.example.com" maxLength={200} ids={ids} />,
    },
    {
      key: "translateTo",
      section: "Lyrics",
      title: "Translate lyrics into",
      description: "The language the Translate button asks for.",
      control: (ids) => select(s, (x) => x.translateTo, TRANSLATE_TO, ids, (v) => updateSettings({ translateTo: v })),
    },
    {
      key: "nowPlayingLyricsCard",
      section: "Lyrics",
      title: "Lyrics card in now playing",
      description: `Shows a song's first lines in the now playing view when ${serverName ?? "your server"} has lyrics.`,
      control: (ids) => (
        <Switch checked={npPrefs.lyricsCard} onChange={(v) => setNpPrefs({ lyricsCard: v })} labelledBy={ids.title} describedBy={ids.desc} />
      ),
    },
    {
      key: "nowPlayingOpenOnStart",
      section: "Appearance",
      title: "Open now playing when a song starts",
      description: "Shows the now playing view on the right, unless the queue is open there.",
      control: (ids) => (
        <Switch checked={npPrefs.openOnStart} onChange={(v) => setNpPrefs({ openOnStart: v })} labelledBy={ids.title} describedBy={ids.desc} />
      ),
    },
    {
      key: "fullWindowHideControls",
      section: "Appearance",
      title: "Expanded view hides controls",
      description: "The expanded view fades its controls after 4 seconds without the mouse.",
      more: "After 4 seconds without moving the mouse, the expanded now-playing view fades its header, the player bar and the top bar, leaving the artwork or lyrics. Moving the mouse or pressing a key brings them back.",
      control: (ids) => toggle(s, "fullWindowHideControls", ids),
    },
    {
      key: "fullWindowRememberMode",
      section: "Appearance",
      title: "Remember the expanded view's mode",
      description: "The expanded view opens on what it showed last. This device only.",
      more: "The expand button opens the expanded view showing what it showed last: artwork, the artist's image or lyrics. Off always starts on the artwork. This device only.",
      control: (ids) => (
        <Switch checked={rememberMode} onChange={fullWindow.setRemember} labelledBy={ids.title} describedBy={ids.desc} />
      ),
    },
    // The miniplayer window
    {
      key: "miniplayerOnTop",
      section: "Appearance",
      title: "Miniplayer always on top",
      description: "The miniplayer stays above other windows. This device only.",
      more: "The miniplayer stays above other windows. Open it from the player bar. It keeps its size and place on this computer. This device only.",
      control: (ids) => toggle(s, "miniplayerOnTop", ids),
    },
    {
      key: "miniplayerTint",
      section: "Appearance",
      title: "Miniplayer background colour",
      description: "The miniplayer takes a dark shade of the cover's colour. This device only.",
      more: "The whole miniplayer takes a dark shade of the cover's colour. Off, it stays dark with the colour on a card behind the cover. Also in the miniplayer's own settings. This device only.",
      control: (ids) => toggle(s, "miniplayerTint", ids),
    },
    {
      key: "miniplayerQueue",
      section: "Appearance",
      title: "Show the queue in the miniplayer",
      description: "Lists what's next under or beside the art. This device only.",
      more: "Lists what's next under the art when the miniplayer is at least 500 tall (turning it on makes a shorter window taller), or beside it when it is wide. Bigger windows list more. This device only.",
      control: (ids) => toggle(s, "miniplayerQueue", ids),
    },
    {
      key: "miniplayerSideBySide",
      section: "Appearance",
      title: "Miniplayer side by side when wide",
      description: "A wide miniplayer puts the art on the left. This device only.",
      more: "A miniplayer much wider than it is tall puts the art on the left and the title, progress and queue beside it. This device only.",
      control: (ids) => toggle(s, "miniplayerSideBySide", ids),
    },
    // Library: the tag editor (the only part that writes to files)
    {
      key: "musicFolder",
      section: "Library",
      page: "tags",
      group: "Tag editor",
      title: "Music folder",
      description: "Your server's music folder, as this computer sees it. This device only.",
      more: "The folder that holds your server's music, as this computer sees it: a mapped drive or network path on Windows, a mounted share on a Mac. Platter edits tags there, in place, and checks that 5 songs are where Plex says. This device only.",
      note: <MusicFolderNote />,
      control: () => <MusicFolderControl />,
      block: () => <MusicFolderPath />,
    },
    {
      key: "rescanAfterTagEdit",
      section: "Library",
      page: "tags",
      group: "Tag editor",
      title: "Rescan in Plex after saving",
      description: "After tags are saved, Plex rescans just that folder so it shows the changes.",
      control: (ids) => toggle(s, "rescanAfterTagEdit", ids),
    },
    {
      key: "askBeforeManyFiles",
      section: "Library",
      page: "tags",
      group: "Tag editor",
      title: "Ask before saving more than 50 files",
      description: "A save that changes more than 50 files asks first.",
      control: (ids) => toggle(s, "askBeforeManyFiles", ids),
    },
    {
      key: "checkTags",
      section: "Library",
      page: "tags",
      group: "Tag editor",
      title: "Check tags",
      description: "Looks for tags worth fixing, each with why it matters and a fix.",
      more: "Looks for tags worth fixing: missing album artists, album details that differ between songs, release types missing a part, disc numbers, release status gaps and more. Each one says why it matters, with a fix when the music folder is set.",
      control: () => <CheckTagsButton />,
    },
    {
      key: "plexLibrarySettings",
      section: "Library",
      page: "tags",
      group: "Tag editor",
      title: "Plex library settings",
      description: "How Plex treats your tags. Read only: change them in Plex.",
      more: "How Plex treats your tags: local metadata preferred, embedded tags read, and the Personal Media agent for albums MusicBrainz doesn't know. Read only: change them in Plex.",
      control: () => null,
      block: () => <PlexLibrarySettings />,
    },
    // Library: tags read from the files
    {
      key: "readFileTags",
      section: "Library",
      page: "tags",
      group: "File tags",
      title: "Read extra tags from files",
      description: "Reads explicit, version and edition tags through Plex. Files never change.",
      more: "Reads explicit, version and edition tags from your music files through Plex, a few kilobytes at a time and only while nothing is downloading for playback. Your files are never changed. This device only.",
      note: <TagScanNote />,
      control: (ids) => toggle(s, "readFileTags", ids),
    },
    {
      key: "explicitContent",
      section: "Library",
      title: "Explicit content",
      description: "Filtered leaves out songs tagged explicit and says how many were skipped.",
      more: "Filtered leaves songs tagged explicit out of albums, playlists and anything else you play, and says how many were skipped. Applies the next time something starts.",
      control: (ids) => select(s, (x) => x.explicitContent, EXPLICIT_OPTIONS, ids, (v) => updateSettings({ explicitContent: v })),
    },
    {
      key: "showFanEdits",
      section: "Library",
      title: "Show fan and AI edits",
      description: "Songs tagged as fan or AI edits show in albums and play.",
      more: "Songs tagged as fan edits or AI edits show in albums and play. Off hides them, and albums made only of them.",
      control: (ids) => toggle(s, "showFanEdits", ids),
    },
    {
      key: "groupEditions",
      section: "Library",
      title: "Group album editions",
      description: "Editions that share a group tag show as one card with an edition picker.",
      more: "Albums that share a group tag, like a deluxe edition or builds of an unreleased album, show as one card. The album page gets an edition picker.",
      control: (ids) => toggle(s, "groupEditions", ids),
    },
    {
      key: "countUnmatchedUnreleased",
      section: "Library",
      page: "tags",
      group: "File tags",
      title: "Count albums Plex couldn't match as unreleased",
      description: "Albums Plex found no match for count as unreleased until you tag them.",
      more: "Albums Plex found no match for, which usually have no release type and no label, are treated as unreleased until you tag them. That covers lyrics transcription, sharing, Discord, mixes and smart playlists.",
      control: (ids) => toggle(s, "countUnmatchedUnreleased", ids),
    },
    {
      key: "checkUnmatchedMusicbrainz",
      section: "Library",
      page: "tags",
      group: "File tags",
      title: "Check MusicBrainz for albums Plex couldn't match",
      description: "If MusicBrainz knows an album Plex missed, it counts as released.",
      more: "Looks each one up on MusicBrainz by artist and title in the background. If MusicBrainz has it, Plex just missed the match and the album counts as released. Albums tagged as demos or unreleased versions are never looked up.",
      note: s && !s.infoMusicbrainz ? "Turn on MusicBrainz on the Artist info sources page to use this." : undefined,
      control: (ids) =>
        s ? (
          <Switch
            checked={s.infoMusicbrainz && s.checkUnmatchedMusicbrainz}
            disabled={!s.infoMusicbrainz}
            onChange={(v) => updateSettings({ checkUnmatchedMusicbrainz: v })}
            labelledBy={ids.title}
            describedBy={ids.desc}
          />
        ) : (
          <Skeleton width={40} height={24} radius="pill" />
        ),
    },
    // Library: the archive (unreleased music, versions and eras)
    {
      key: "archiveFolderKeywords",
      section: "Library",
      page: "tags",
      group: "Archive",
      title: "Folder names that hint at unreleased music",
      description: "A folder name adds weight but never decides on its own.",
      more: "When a song's folder has one of these names, Why? lists it for albums that are probably unreleased. A folder name adds weight but never decides on its own. Leave the list empty to stop using folder names.",
      control: (ids) => (s ? <FolderKeywords settings={s} ids={ids} /> : <Skeleton width={240} height={32} />),
    },
    {
      key: "releaseStatusTag",
      section: "Library",
      page: "tags",
      group: "Archive",
      title: "Release status tag",
      description: "Where Platter reads whether a song is released, unreleased or another version.",
      more: "Where Platter reads whether a song is released, unreleased, leaked or an alternate version. Auto reads Platter's own ARCHIVE_VERSION, then a STATUS tag, then the release status MusicBrainz Picard writes. A custom tag is read after ARCHIVE_VERSION instead. Changing the tag reads your files again in the background.",
      control: (ids) => (s ? <ReleaseStatusTag settings={s} ids={ids} /> : <Skeleton width={240} height={32} />),
    },
    {
      key: "releaseStatusValues",
      section: "Library",
      page: "tags",
      group: "Archive",
      title: "Release status values",
      description: "Values your tags use and what each means. Unknown ones never count as released.",
      more: "Values your tags use and what each one means. Common ones like Released, Official, Unreleased, Leak, Alt, Demo and OG file are known already. A value Platter doesn't know never counts as released.",
      control: (ids) => (s ? <ReleaseStatusValues settings={s} ids={ids} /> : <Skeleton width={240} height={32} />),
    },
    {
      key: "showVersionCounts",
      section: "Library",
      page: "tags",
      group: "Archive",
      title: "Show other versions on songs",
      description: "Released songs with other versions show +N versions next to the title.",
      more: "Released songs with demos, leaks or other versions in your library show +N versions next to the title. Other versions stays in the song's menu either way.",
      control: (ids) => toggle(s, "showVersionCounts", ids),
    },
    {
      key: "showEraCards",
      section: "Library",
      page: "tags",
      group: "Archive",
      title: "Show era cards on artist pages",
      description: "Artists with two or more tagged eras get era cards under Popular.",
      more: "Artists whose songs are tagged with two or more eras get a row of era cards under Popular. Picking one shows only that era, released and unreleased.",
      control: (ids) => toggle(s, "showEraCards", ids),
    },
    {
      key: "showLineage",
      section: "Library",
      page: "tags",
      group: "Archive",
      title: "Show lineage",
      description: "Every version of a song on one timeline, with Compare.",
      more: "Songs with other versions get Lineage in their menu, in Other versions and in Your stats: every version on one timeline, from the first demo to the live takes, with Compare to switch between two at the same moment.",
      control: (ids) => toggle(s, "showLineage", ids),
    },
    {
      key: "compareLevelMatch",
      section: "Library",
      page: "tags",
      group: "Archive",
      title: "Compare versions at the same loudness",
      description: "Compare plays both versions at the same loudness.",
      more: "Compare plays both versions at the same loudness when your server has loudness data for them, so the louder one doesn't sound better just for being louder. Your own sound settings come back when Compare closes.",
      control: (ids) => toggle(s, "compareLevelMatch", ids),
    },
    // Library: smart playlists
    {
      key: "showDescribePlaylist",
      section: "Library",
      title: "Show \"Describe a playlist\"",
      description: "Type what you want and it becomes smart playlist rules. Nothing is sent.",
      more: "Type what you want, like \"chill late-night stuff from 2016 I haven't played lately\", and it becomes smart playlist rules you can change before saving. It's read on this computer from your library's own names; nothing is sent anywhere.",
      control: (ids) => toggle(s, "showDescribePlaylist", ids),
    },
    // Library: the sound map
    {
      key: "showSoundMap",
      section: "Library",
      title: "Show the sound map",
      description: "A map of your library where songs that sound alike sit together.",
      more: "A map of your library where songs that sound alike sit together, in Explore on Home once songs are analysed. It fills in as Automix analyses songs on this computer and works offline.",
      note: s && !s.automix ? "Songs are analysed while Automix is on." : undefined,
      control: (ids) => toggle(s, "showSoundMap", ids),
    },
    // Library: the library doctor
    {
      key: "libraryDoctor",
      section: "Library",
      page: "doctor",
      title: "Check the library in the background",
      description: "Lists what's worth fixing, with a fix for each. Nothing changes until you confirm.",
      more: "Checks your library in the background, after each sync and when new tags are read, and lists what's worth fixing with a fix for each. Nothing changes until you pick a fix and confirm it. The count shows under Your Library.",
      note: <DoctorNote on={s?.libraryDoctor !== false} />,
      control: (ids) => toggle(s, "libraryDoctor", ids),
    },
    {
      key: "doctorChecks",
      section: "Library",
      page: "doctor",
      title: "What to check",
      description: "Turning one off takes its findings out of the list and the count.",
      more: "Duplicate files, missing or small covers, album tags that differ, albums split in two, missing album artists, songs you have a better copy of, albums Plex didn't match, and songs that sound like another recording. Turning one off takes its findings out of the list and the count.",
      control: () => null,
      block: () => (s ? <DoctorChecksList settings={s} /> : <Skeleton width={240} height={32} />),
    },
    {
      key: "doctorCoverPx",
      section: "Library",
      page: "doctor",
      title: "Small cover size",
      description: "Covers smaller than this on their longest side count as small.",
      more: "Covers smaller than this on their longest side count as small. Covers look soft in full window below about 600 pixels.",
      control: (ids) => select(s, (x) => String(x.doctorCoverPx ?? 600), COVER_PX_OPTIONS, ids, (v) => updateSettings({ doctorCoverPx: Number(v) })),
    },
    // Library: identify by sound
    {
      key: "identifyBySound",
      section: "Library",
      page: "soundId",
      title: "Identify songs by sound",
      description: "Names songs by their sound, with AcoustID, when their tags don't. This device only.",
      more: SOUND_ID_DESCRIPTION,
      note: rowNote(soundIdNote(soundId, s?.identifyBySound)),
      control: (ids) => toggle(s, "identifyBySound", ids),
    },
    {
      key: "acoustidKey",
      section: "Library",
      page: "soundId",
      title: "Your own AcoustID key",
      description: "Optional. Your own free AcoustID key, kept in this computer's keychain.",
      more: "Optional. An application key you register for free on the AcoustID website, used instead of the one Platter comes with. Kept in this computer's keychain.",
      control: (ids) => <AcoustIdKeyControl ids={ids} status={soundId} setKey={setAcoustIdKey} />,
    },
    {
      key: "soundIdScope",
      section: "Library",
      page: "soundId",
      title: "Which songs",
      description: "Untagged songs only, or every song.",
      more: "Songs whose files have no MusicBrainz recording id or ISRC, or every song. All songs takes longer and reads more from your server, a little at a time while nothing is playing.",
      control: (ids) =>
        select(s, (x) => x.soundIdScope ?? "untagged", SOUND_ID_SCOPE_OPTIONS, ids, (v) => void updateSettings({ soundIdScope: v })),
    },
    // Library: drag and drop
    {
      key: "dragAndDrop",
      section: "Library",
      title: "Drag and drop to add to playlists and the queue",
      description: "Drag songs, albums and playlists onto a playlist or into the queue.",
      more: "Drag songs, albums and playlists onto a playlist or Liked Songs in Your Library, or into the queue. Reordering a playlist or the queue by dragging works either way.",
      control: (ids) => toggle(s, "dragAndDrop", ids),
    },
    // Lyrics transcribed on this computer
    {
      key: "transcribeLyrics",
      section: "Lyrics",
      title: "Write down lyrics for songs without them",
      description: "Done on this computer as songs play; the audio never leaves it. This device only.",
      more: "When a song has no lyrics anywhere, Platter writes them down as it plays, and does the next songs in the queue ahead of time. A speech model downloads once, the first time it's needed. Results can have mistakes and can be fixed with Edit lyrics. This device only.",
      note: <TranscribeNote />,
      control: (ids) => toggle(s, "transcribeLyrics", ids),
    },
    {
      key: "transcribePauseBusy",
      section: "Lyrics",
      page: "transcription",
      title: "Pause while the computer is busy",
      description: "Background work waits while other programs are busy. This device only.",
      more: "Albums, the background batch and the lyrics check wait while other programs keep the processor busy or a game is fullscreen. A song you ask for still runs. This device only.",
      control: (ids) => toggle(s, "transcribePauseBusy", ids),
    },
    {
      key: "transcribeFasterAway",
      section: "Lyrics",
      page: "transcription",
      title: "Work faster while I'm away",
      description: "Uses a little more of the processor while you're away. This device only.",
      more: "After 5 minutes without the keyboard or mouse, and while plugged in, background work uses a little more of the processor. It goes back as soon as you're back. This device only.",
      control: (ids) => toggle(s, "transcribeFasterAway", ids),
    },
    {
      key: "transcribeUnreleased",
      section: "Lyrics",
      page: "transcription",
      title: "Transcribe all unreleased music without lyrics",
      description: "Works through them in the background, resting between songs.",
      more: "Works through them in the background with fewer processor cores, resting a few seconds between songs. It pauses on battery power and always gives way to playback. Pause keeps your place, even after a restart. If you pick another model, songs carry on with the one you have until the new one is downloaded, and Redo does them again with it. Away from home it reads songs while playback has nothing to download, and songs you just played come from this computer.",
      note: <BatchNote />,
      control: () => <BatchControl />,
    },
    {
      key: "showAutoLyrics",
      section: "Lyrics",
      page: "transcription",
      title: "Show auto-transcribed lyrics",
      description: "Off hides transcribed lyrics you haven't edited. They stay saved.",
      control: (ids) => toggle(s, "showAutoLyrics", ids),
    },
    {
      key: "syncTranscriptions",
      section: "Lyrics",
      page: "transcription",
      title: "Sync transcriptions to my devices",
      description: "Each song is transcribed once across your computers.",
      more: "Transcribed lyrics go to your other devices through the sync file, so each song is transcribed once. Lyrics you wrote always come first. Needs Lyrics you wrote on, on the Sync page.",
      control: (ids) => toggle(s, "syncTranscriptions", ids),
    },
    {
      key: "shareTranscriptions",
      section: "Lyrics",
      page: "transcription",
      title: "Offer to share transcriptions publicly",
      description: "After you check one, offers to share it to LRCLIB. Asks every time.",
      more: "After you check a transcription, the lyrics view offers to share it to LRCLIB, a free public lyrics library. It shows what is sent and asks every time.",
      control: (ids) => toggle(s, "shareTranscriptions", ids),
    },
    {
      key: "shareTranscriptionsUnreleased",
      section: "Sharing and friends",
      group: "Unreleased music, feature by feature",
      title: "Share transcriptions of unreleased music",
      description: "Sharing publishes the song's title, so it becomes public knowledge.",
      more: "Sharing publishes the song's title, so an unreleased song becomes public knowledge.",
      control: (ids) => toggle(s, "shareTranscriptionsUnreleased", ids),
    },
  ];


  const usable = rows.filter((r) => (!r.cap || caps[r.cap]) && (!isPhone || onPhone(r))).map((r) => (isPhone ? phoneWords(r) : r));
  const q = query.trim();
  const inline = settingsView.inlinePages;
  const transcribing = !!s?.transcribeLyrics;
  /** Lyrics transcription has a page only while transcription is on. */
  const pageOn = (id: PageId) => id !== "transcription" || transcribing;
  const page = pageMeta(pageProp ?? routePage ?? usable.find((r) => r.key === routeRow)?.page);

  /** Whether each integration is set up (its card's status). */
  const connected: Partial<Record<PageId, boolean>> = {
    lidarr: lidarr.ready,
    discord: discordOn,
    scrobbling: !!scrobble && (scrobble.lastfm.signedIn || (!!s?.scrobbleListenbrainz && scrobble.listenbrainz.hasToken)),
    artistInfo:
      !!s &&
      (s.infoWikipedia || s.infoMusicbrainz || s.infoListenbrainz || s.infoListenbrainzStats || !!infoKeys?.lastfm.saved || !!infoKeys?.fanart.saved || !!infoKeys?.audiodb.saved),
    discogs: !!discogs?.saved,
    tautulli: !!history.source("tautulli")?.setUp,
    ai: !!s && s.aiProvider !== "none" && !!ai?.hasKey,
    streamDeck: !!s?.allowStreamDeck || !!s?.allowAutomation,
    widgetFeed: !!s?.nowPlayingFeed,
    bridge: !!bridge?.bridges.length,
    libraryImport: !!s?.importSources?.length,
  };

  /** Pages that hold whole sections of their own, and when search shows them. */
  const extras: Partial<Record<PageId, { matches: (q: string) => boolean; render: () => ReactNode; first?: boolean }>> = {
    libraryImport: {
      matches: (x) => importMatches(x) || matchesText('Show "Played a lot" on the Missing page. Bring in my listening.', x),
      render: () => <LibraryImportSettings />,
    },
    streamDeck: {
      matches: (x) => streamDeckMatches(x) || automationMatches(x) || matchesText("Platter plugin. Allow Stream Deck. Allow automation.", x),
      render: () => (
        <>
          <StreamDeckSettings />
          <AutomationSettings />
        </>
      ),
    },
    // The Bridge's card comes first; its switches sit under More below it.
    bridge: { matches: bridgeMatches, render: () => <BridgeSettings />, first: true },
    tags: { matches: (x) => !!caps.metadataEdit && plexEditMatches(x), render: () => (caps.metadataEdit ? <PlexEditSettings /> : null) },
  };

  /** Buttons a Manage row carries besides Manage itself. */
  const manageActions: Partial<Record<PageId, ReactNode>> = {
    tags: <CheckTagsButton />,
    doctor: (
      <Button size="sm" onClick={() => nav.go({ name: "doctor" })} disabled={s?.libraryDoctor === false} aria-label="Open Library doctor">
        Open
      </Button>
    ),
    looks: (
      <Button size="sm" onClick={() => looks.openPick()}>
        Pick a look
      </Button>
    ),
  };

  const openPage = (id: PageId) => nav.go({ name: "settings", page: id });
  const manageRow = (p: PageMeta): RowDef => ({
    key: `manage-${p.id}`,
    section: p.group,
    title: p.title,
    description: p.blurb,
    control: () => (
      <>
        {manageActions[p.id]}
        <Button size="sm" onClick={() => openPage(p.id)} aria-label={`Manage ${p.title}`}>
          Manage
        </Button>
      </>
    ),
  });

  /** The settings a section's (or page's) rows control (Reset puts them back). */
  const scopeRows = (scope: string) =>
    scope.startsWith("page:") ? usable.filter((r) => r.page === scope.slice(5)) : usable.filter((r) => r.section === scope);
  const scopeSettings = (scope: string) => (s ? settingsOfRows(scopeRows(scope).map((r) => r.key), s) : []);
  const scopeTitle = (scope: string) => (scope.startsWith("page:") ? (pageMeta(scope.slice(5))?.title ?? "") : scope);
  const reset = async (scope: string) => {
    const title = scopeTitle(scope);
    try {
      const defaults = await backend.settingsDefaults();
      await updateSettings(resetPatch(scopeSettings(scope), defaults));
      toast.info(`${title} settings are back to how they start`);
    } catch (e) {
      toast.error(failText(`Couldn't reset ${title}`, e));
    }
  };

  // ------------------------------------------------------------ nav
  const [active, setActive] = useState<GroupName>(GROUPS[0]);
  const pendingScroll = useRef<GroupName | null>(null);
  const shownGroup = page?.group;
  /** Until when the nav keeps the group just clicked, while the scroll it started settles. */
  const navLock = useRef(0);
  const scrollToGroup = (g: GroupName) => {
    const heading = document.getElementById(groupId(g));
    if (!heading) return;
    navLock.current = Date.now() + 1000;
    heading.closest("section")?.scrollIntoView?.({ block: "start" });
    heading.focus({ preventScroll: true });
  };
  const goTo = (g: GroupName) => {
    setActive(g);
    if (searching) setQuery("");
    if (page || searching) {
      pendingScroll.current = g;
      if (page) nav.go({ name: "settings" });
      return;
    }
    scrollToGroup(g);
  };
  useEffect(() => {
    if (page || searching || !pendingScroll.current) return;
    const g = pendingScroll.current;
    pendingScroll.current = null;
    scrollToGroup(g);
  });
  // Sections can come and go (settings loading, a capability turning up):
  // the observer follows the ones on screen.
  const sectionsKey = s ? usable.map((r) => r.section).join("|") : "";
  useEffect(() => {
    if (page || searching || typeof IntersectionObserver === "undefined") return;
    const seen = new Map<string, boolean>();
    // The band near the top of the scrolling area, under the page header that floats over it.
    const sections = document.querySelectorAll<HTMLElement>(".settings__group[data-group]");
    let root: HTMLElement | null = sections[0]?.parentElement ?? null;
    while (root && !/(auto|scroll)/.test(getComputedStyle(root).overflowY)) root = root.parentElement;
    const shown = GROUPS.filter((g) => [...sections].some((el) => el.dataset.group === g));
    const pick = () => {
      if (Date.now() < navLock.current) return;
      // Scrolled to the end: the last groups are too short to reach the band.
      if (root && root.scrollHeight > root.clientHeight && root.scrollTop + root.clientHeight >= root.scrollHeight - 2) {
        const last = shown[shown.length - 1];
        if (last) setActive(last);
        return;
      }
      const first = GROUPS.find((g) => seen.get(g));
      if (first) setActive(first);
    };
    const io = new IntersectionObserver(
      (entries) => {
        for (const e of entries) seen.set((e.target as HTMLElement).dataset.group ?? "", e.isIntersecting);
        pick();
      },
      { root, rootMargin: "-80px 0px -60% 0px" },
    );
    sections.forEach((el) => io.observe(el));
    const onScroll = () => pick();
    root?.addEventListener("scroll", onScroll, { passive: true });
    return () => {
      io.disconnect?.();
      root?.removeEventListener("scroll", onScroll);
    };
  }, [page, searching, sectionsKey]);
  const current = shownGroup ?? active;

  const backToSettings = () => {
    if (prevRoute?.name === "settings" && !prevRoute.page) nav.back();
    else nav.go({ name: "settings" });
  };

  // ------------------------------------------------------------ parts
  /** The linked row whose More was opened (once, so its toggle still works). */
  const seededRow = useRef<string | undefined>(undefined);
  useEffect(() => {
    if (!routeRow || seededRow.current === routeRow) return;
    const r = usable.find((x) => x.key === routeRow);
    if (!r || !inMore(r)) return;
    seededRow.current = routeRow;
    const scope = r.page ? `page:${r.page}` : r.section;
    setOpenAdvanced((prev) => {
      const next = new Set(prev);
      if (allOpen) next.delete(scope);
      else next.add(scope);
      return next;
    });
  });
  const toggleAdvanced = (scope: string) =>
    setOpenAdvanced((prev) => {
      const next = new Set(prev);
      if (next.has(scope)) next.delete(scope);
      else next.add(scope);
      if (!allOpen) saveMoreOpen(next);
      return next;
    });
  const moreOpen = (scope: string) => allOpen !== openAdvanced.has(scope);

  /**
   * Rows: the common ones, then one More at the end that opens the rest in
   * place (remembered per section). `between` sits above More; `hidden` is
   * extra content under it (integrations not set up), with how many it holds.
   */
  const foldedRows = (scope: string, list: RowDef[], domId: string, between?: ReactNode, hidden?: { node: ReactNode; count: number }) => {
    const everyday = list.filter((r) => !inMore(r));
    const advanced = list.filter((r) => inMore(r));
    const count = advanced.length + (hidden?.count ?? 0);
    // A search shows matching rows without a click, and so does a link to one.
    // A link to a row under More opens it until the effect above takes over; the toggle works after.
    const open = searching || moreOpen(scope) || (advanced.some((r) => r.key === routeRow) && seededRow.current !== routeRow);
    return (
      <>
        <RowList rows={everyday} />
        {between}
        {count > 0 && (
          <div className="settings__advanced">
            <button
              type="button"
              className="settings__advanced-toggle"
              aria-expanded={open}
              aria-controls={`${domId}-advanced`}
              aria-label={`More in ${scopeTitle(scope)}, ${count} ${count === 1 ? "setting" : "settings"}`}
              disabled={searching}
              onClick={() => toggleAdvanced(scope)}
            >
              <ChevronRight size={16} aria-hidden="true" className="settings__advanced-chevron" />
              More
              <span className="settings__advanced-count">{count}</span>
            </button>
            <div id={`${domId}-advanced`} hidden={!open}>
              {open && (
                <>
                  <RowList rows={advanced} />
                  {hidden?.node}
                </>
              )}
            </div>
          </div>
        )}
      </>
    );
  };

  const sectionHead = (scope: string, title: string, domId: string) => {
    const more = GROUP_HELP[title];
    return (
      <div className="settings__head">
        <h2 id={domId} className="settings__heading" tabIndex={-1}>
          {title}
        </h2>
        {more && <HelpButton page={more[0]} anchor={more[1]} label={`Help with ${title}`} />}
        {!searching && !isPhone && scopeSettings(scope).length > 0 && <GroupMenu title={title} onReset={() => setResetSection(scope)} />}
      </div>
    );
  };

  /** A page's rows shown in place (tests, and search results). */
  const inlinePage = (p: PageMeta, list: RowDef[], withExtras: boolean) => (
    <div key={p.id} className="settings__inline-page" data-page={p.id}>
      <div className="help-heading">
        <h3 className="settings__subheading">{p.title}</h3>
        {GROUP_HELP[p.title] && <HelpButton page={GROUP_HELP[p.title][0]} anchor={GROUP_HELP[p.title][1]} label={`Help with ${p.title}`} />}
        {searching && (
          <Button size="sm" variant="ghost" onClick={() => openPage(p.id)} aria-label={`Open ${p.title}`}>
            Open
          </Button>
        )}
      </div>
      <RowList rows={list} markMore={searching} />
      {withExtras && extras[p.id]?.render()}
    </div>
  );

  /** A group's pages: integrations set up and everyday tools show; the rest go under More. */
  const splitPages = (pages: PageMeta[]) => {
    const under = (p: PageMeta) => (p.kind === "integration" ? !connected[p.id] : MANAGE_IN_MORE.has(p.id));
    return { shown: pages.filter((p) => !under(p)), more: pages.filter(under) };
  };

  const groupPages = (g: GroupName) => PAGES.filter((p) => p.group === g && pageOn(p.id) && (!isPhone || !DESKTOP_ONLY_PAGES.has(p.id)));

  /** One group on the main view. */
  const groupSection = (g: GroupName) => {
    const domId = groupId(g);
    const list = usable.filter((r) => r.section === g && !r.page);
    const pages = groupPages(g);
    const { shown: shownPages, more: morePages } = splitPages(pages);
    const cards = shownPages.filter((p) => p.kind === "integration");
    const manage = shownPages.filter((p) => p.kind === "manage");
    const moreCards = morePages.filter((p) => p.kind === "integration");
    const moreManage = morePages.filter((p) => p.kind === "manage");
    const noneSetUp = moreCards.length > 0 && cards.length === 0;
    return (
      <section key={g} id={`${domId}-section`} className="settings__group" aria-labelledby={domId} data-group={g}>
        {sectionHead(g, g, domId)}
        {foldedRows(
          g,
          list,
          domId,
          <>
            {cards.length > 0 && (
              <div className="settings-cards">
                {cards.map((p) => (
                  <IntegrationCard key={p.id} p={p} on={!!connected[p.id]} onOpen={() => openPage(p.id)} />
                ))}
              </div>
            )}
            {noneSetUp && (
              <div className="settings__add">
                <p className="setting-row__desc">Nothing is set up here yet.</p>
                {!moreOpen(g) && (
                  <Button size="sm" onClick={() => toggleAdvanced(g)}>
                    Add an integration
                  </Button>
                )}
              </div>
            )}
            {manage.length > 0 && (
              <div className="settings__manage">
                <RowList rows={manage.map(manageRow)} />
              </div>
            )}
          </>,
          morePages.length > 0
            ? {
                count: morePages.length,
                node: (
                  <>
                    {moreCards.length > 0 && (
                      <div className="settings-cards">
                        {moreCards.map((p) => (
                          <IntegrationCard key={p.id} p={p} on={!!connected[p.id]} onOpen={() => openPage(p.id)} />
                        ))}
                      </div>
                    )}
                    {moreManage.length > 0 && (
                      <div className="settings__manage">
                        <RowList rows={moreManage.map(manageRow)} />
                      </div>
                    )}
                  </>
                ),
              }
            : undefined,
        )}
        {inline &&
          pages.map((p) =>
            inlinePage(
              p,
              usable.filter((r) => r.page === p.id && r.shown !== false),
              true,
            ),
          )}
      </section>
    );
  };

  /** Search results: matching rows from every group and page, those under More included (and marked). */
  const results = () => {
    const out: ReactNode[] = [];
    for (const g of GROUPS) {
      const own = usable.filter((r) => r.section === g && !r.page && matches(r, q));
      const pages = groupPages(g)
        .map((p) => {
          const rowsOf = usable.filter((r) => r.page === p.id);
          const hits = rowsOf.filter((r) => r.shown !== false && matches(r, q));
          // A row waiting for its service, or the page's own name, offers the page.
          const waiting = rowsOf.some((r) => r.shown === false && matches(r, q));
          const named = matchesText(`${p.title} ${p.blurb}`, q);
          const extra = extras[p.id]?.matches(q) ?? false;
          return { p, hits, offer: (waiting || named) && !hits.length && !extra, extra };
        })
        .filter((x) => x.hits.length || x.offer || x.extra);
      if (!own.length && !pages.length) continue;
      const domId = groupId(g);
      out.push(
        <section key={g} id={`${domId}-section`} className="settings__group" aria-labelledby={domId} data-group={g}>
          {sectionHead(g, g, domId)}
          <RowList rows={own} markMore />
          {pages.some((x) => x.offer) && (
            <div className="settings__manage">
              <RowList rows={pages.filter((x) => x.offer).map((x) => manageRow(x.p))} />
            </div>
          )}
          {pages.filter((x) => !x.offer).map((x) => inlinePage(x.p, x.hits, x.extra))}
        </section>,
      );
    }
    return out;
  };

  /** One page: an integration or a tool moved out of its group. */
  const pageView = (p: PageMeta) => {
    const domId = `settings-page-${p.id}`;
    const list = usable.filter((r) => r.page === p.id && r.shown !== false);
    const waiting = usable.some((r) => r.page === p.id && r.shown === false);
    const on = connected[p.id];
    return (
      <section className="settings__group settings__page" aria-labelledby={domId} data-page={p.id}>
        <button type="button" className="settings__back" onClick={backToSettings}>
          <ChevronLeft size={16} aria-hidden="true" />
          {p.group}
        </button>
        {sectionHead(`page:${p.id}`, p.title, domId)}
        <p className="settings__page-blurb">
          {p.blurb}
          {p.kind === "integration" && (
            <span className={`settings-card__status${on ? " is-on" : ""}`}>{on ? "Connected" : "Not set up"}</span>
          )}
        </p>
        {!pageOn(p.id) ? (
          <p className="setting-row__desc" role="note">
            Turn on Transcribe songs without lyrics on this computer, under Lyrics, to use these.
          </p>
        ) : (
          <>
            {extras[p.id]?.first && extras[p.id]?.render()}
            {foldedRows(`page:${p.id}`, list, domId)}
            {waiting && (
              <p className="setting-row__desc settings__waiting" role="note">
                More settings show once this is connected.
              </p>
            )}
            {!extras[p.id]?.first && extras[p.id]?.render()}
          </>
        )}
      </section>
    );
  };

  const anyResult = searching
    ? GROUPS.some(
        (g) =>
          usable.some((r) => r.section === g && (r.page ? pageOn(r.page) : true) && matches(r, q)) ||
          groupPages(g).some((p) => matchesText(`${p.title} ${p.blurb}`, q) || (extras[p.id]?.matches(q) ?? false)),
      )
    : true;

  const dialogs = (
    <>
      <ConfirmDialog
        open={dialog === "signOut"}
        title="Sign out of Plex?"
        body={`Platter forgets your Plex sign-in on this ${isPhone ? "phone" : "computer"}. You'll need to sign in again to play your music.`}
        confirm="Sign out"
        onConfirm={() => void signOut()}
        onClose={() => setDialog(null)}
      />
      <ConfirmDialog
        open={removeHistory !== null}
        title={`Remove your plays from ${removeHistory ? SOURCE_NAMES[removeHistory] : ""}?`}
        body={`Platter deletes the plays it brought in from ${removeHistory ? SOURCE_NAMES[removeHistory] : ""} and turns this source off. Nothing changes there, and Platter's own plays stay. Turn it on again to bring them back.`}
        confirm="Remove plays"
        onConfirm={() => removeHistory && void history.remove(removeHistory)}
        onClose={() => setRemoveHistory(null)}
      />
      <ConfirmDialog
        open={resetSection !== null}
        title={`Reset ${resetSection ? scopeTitle(resetSection) : ""} settings?`}
        body={`The ${resetSection ? scopeTitle(resetSection) : ""} settings go back to how they are on a new install. Addresses, folders and keys you set up stay as they are.`}
        confirm="Reset"
        onConfirm={() => resetSection && void reset(resetSection)}
        onClose={() => setResetSection(null)}
      />
      <ConfirmDialog
        open={pendingRule !== null}
        title="Undo your exceptions?"
        body={`${pendingRule?.lost === 1 ? "1 feature is" : `${pendingRule?.lost ?? 0} features are`} set differently under More. This rule sets ${pendingRule?.lost === 1 ? "it" : "them"} back.`}
        confirm="Change the rule"
        onConfirm={() => pendingRule && void updateSettings(patchForRule(pendingRule.rule, useSettings.getState().settings))}
        onClose={() => setPendingRule(null)}
      />
      <ServerDialog open={dialog === "server"} onClose={() => setDialog(null)} current={auth?.server?.machineId} />
      <NoticesDialog open={dialog === "notices"} onClose={() => setDialog(null)} />
    </>
  );

  if (isPhone) {
    const g = page ? undefined : (sectionProp ?? routeSection);
    // phoneScrollKey is read by the effect above, which keeps each list's place.
    phoneScrollKey.current = page ? `page:${page.id}` : (g ?? "");
    const offline = usable.filter((r) => r.key === "goOffline");
    let body: ReactNode;
    if (page) body = pageView(page);
    else if (g) {
      const domId = groupId(g);
      const { shown: shownPages, more: morePages } = splitPages(groupPages(g));
      const pageList = (list: PageMeta[], label: string) => (
        <ul className="ph-set-list ph-set-list--pages" aria-label={label}>
          {list.map((p) => (
            <li key={p.id}>
              <button type="button" className="ph-set-link" onClick={() => openPage(p.id)} data-page={p.id}>
                <span className="ph-set-link__text">
                  <span className="ph-set-link__title">{p.title}</span>
                  <span className="ph-set-link__sub">{p.blurb}</span>
                </span>
                {p.kind === "integration" && <span className={`ph-set-link__state${connected[p.id] ? " is-on" : ""}`}>{connected[p.id] ? "On" : "Off"}</span>}
                <ChevronRight size={18} aria-hidden="true" className="ph-set-link__chev" />
              </button>
            </li>
          ))}
        </ul>
      );
      body = (
        <section className="settings__group ph-set-section" aria-labelledby={domId} data-group={g}>
          <h1 id={domId} className="ph-title" tabIndex={-1}>
            {g}
          </h1>
          {foldedRows(
            g,
            // The phone's Look leads Appearance, where Theme stands on a computer.
            usable.filter((r) => r.section === g && !r.page).sort((a, b) => Number(b.key === "phoneLook") - Number(a.key === "phoneLook")),
            domId,
            shownPages.length > 0 && pageList(shownPages, `Pages in ${g}`),
            morePages.length > 0 ? { count: morePages.length, node: pageList(morePages, `More pages in ${g}`) } : undefined,
          )}
          {scopeSettings(g).length > 0 && (
            <button type="button" className="ph-set-reset" onClick={() => setResetSection(g)}>
              Reset {g} settings
            </button>
          )}
        </section>
      );
    } else
      body = (
        <>
          <h1 className="ph-title">Settings</h1>
          <label className="ph-field ph-set-search">
            <span className="sr-only">Search settings</span>
            <input
              type="search"
              className="ph-field__input"
              value={query}
              placeholder="Search settings"
              enterKeyHint="search"
              autoCorrect="off"
              autoCapitalize="none"
              onChange={(e) => setQuery(e.target.value)}
            />
          </label>
          {searching ? (
            <>
              {results()}
              {!anyResult && (
                <div className="ph-empty" role="status">
                  <p className="ph-empty__title">No settings match "{q}"</p>
                  <p className="ph-empty__text">Try another word, like quality, cache or motion.</p>
                </div>
              )}
            </>
          ) : (
            <>
              {offline.length > 0 && (
                <div className="ph-set-card">
                  <RowList rows={offline} />
                </div>
              )}
              <ul className="ph-set-list" aria-label="Settings sections">
                {GROUPS.filter((x) => usable.some((r) => r.section === x) || groupPages(x).length > 0).map((x) => (
                  <li key={x}>
                    <button type="button" className="ph-set-link" onClick={() => nav.go({ name: "settings", section: x })}>
                      <span className="ph-set-link__text">
                        <span className="ph-set-link__title">{x}</span>
                        <span className="ph-set-link__sub">{GROUP_BLURB[x]}</span>
                      </span>
                      <ChevronRight size={18} aria-hidden="true" className="ph-set-link__chev" />
                    </button>
                  </li>
                ))}
              </ul>
            </>
          )}
        </>
      );
    return (
      <div className="settings settings--phone" ref={phoneRoot}>
        {body}
        {dialogs}
      </div>
    );
  }

  return (
    <div className="page-pad settings">
      <h1 className="page-title">Settings</h1>
      <div className="settings__layout">
        <nav className="settings__nav" aria-label="Settings sections">
          <ul className="settings__nav-list">
            {GROUPS.map((g) => (
              <li key={g}>
                <button
                  type="button"
                  className="settings__nav-link"
                  aria-current={g === current && !searching ? "true" : undefined}
                  onClick={() => goTo(g)}
                >
                  {g}
                </button>
              </li>
            ))}
          </ul>
          <div className="settings__jump">
            <Select value={current} options={GROUPS.map((g) => ({ value: g, label: g }))} onChange={goTo} label="Go to a section" />
          </div>
        </nav>
        <div className="settings__main">
          <div className="settings__search">
            <SearchField
              value={query}
              onChange={(v) => {
                setQuery(v);
                if (v.trim() && page) nav.go({ name: "settings" });
              }}
              label="Search settings"
              placeholder="Search settings"
            />
          </div>
          {searching ? results() : page ? pageView(page) : GROUPS.map(groupSection)}
          {!anyResult && (
            <div className="settings__empty" role="status">
              <p className="settings__empty-title">No settings match "{q}"</p>
              <p className="settings__empty-hint">Try another word, like quality, cache or motion.</p>
            </div>
          )}
          <p className="settings__hint">Shortcut: {modKey}+,</p>
        </div>
      </div>
      {dialogs}
    </div>
  );
}
