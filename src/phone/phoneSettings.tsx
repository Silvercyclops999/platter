/**
 * The phone's own settings rows (Downloads and devices, "On this phone",
 * "CarPlay" and "Siri").
 * They show only when Platter runs as a phone; all but Siri's are this
 * device only.
 */
import { Select, type SelectOption } from "../components/Select";
import { Skeleton } from "../components/Skeleton";
import { Switch } from "../components/Switch";
import { PHONE_KEEP_RECENT_CHOICES, type MobileQuality, type Settings } from "../lib/types";
import type { RowDef } from "../pages/SettingsPage";
import { updateSettings } from "../stores/settings";

export const MOBILE_QUALITY: SelectOption<MobileQuality>[] = [
  { value: "auto", label: "Automatic" },
  { value: "original", label: "Original" },
  { value: "320", label: "320 kbps" },
  { value: "256", label: "256 kbps" },
  { value: "192", label: "192 kbps" },
  { value: "128", label: "128 kbps" },
];

export const KEEP_RECENT: SelectOption<string>[] = PHONE_KEEP_RECENT_CHOICES.map((n) => ({
  value: String(n),
  label: n === 0 ? "None" : `${n} songs`,
}));

type Ids = { title: string; desc: string };
const wait = <Skeleton width={40} height={24} radius="pill" />;

type PhoneSwitch = "phoneMobileDownloads" | "phoneHaptics" | "carplayResume" | "carplayOfflineDownloads";

function sw(s: Settings | null, key: PhoneSwitch, ids: Ids) {
  if (!s) return wait;
  return <Switch checked={s[key]} onChange={(v) => updateSettings({ [key]: v })} labelledBy={ids.title} describedBy={ids.desc} />;
}

const GROUP = "On this phone";
const CARPLAY = "CarPlay";
const SIRI = "Siri";

export const SIRI_DISLIKE: SelectOption<string>[] = [
  { value: "hide", label: "Hide it and skip" },
  { value: "skip", label: "Just skip" },
];

/** What to say, for the "Ask Siri" row. */
export const SIRI_PHRASES =
  'End with "on Platter". "Play The Beatles on shuffle", "play the album Abbey Road", "play my Road Trip playlist", ' +
  '"play Daily Mix 2", "play hip-hop", "play the newest album by The Beatles", "play something", "play Yesterday next", ' +
  '"add Abbey Road to the queue", "play more like this", "I like this song", "I don\'t like this song", ' +
  '"add this to my Road Trip playlist". Pause, skip and "turn on shuffle" work too. Songs you don\'t have are ' +
  "got through Lidarr when it's set up.";

/** The phone's one look choice: follow the phone's light or dark, or one look. */
export type PhoneLook = "device" | "studio" | "studioBlack" | "sleeve" | "other";

export function phoneLookOf(s: Pick<Settings, "theme" | "followSystem" | "themeFile">): PhoneLook {
  if (s.themeFile) return "other";
  const studio = s.theme === "studio" || s.theme === "studioBlack" || s.theme === "sleeve";
  if (!studio) return "other";
  if (s.followSystem) return "device";
  return s.theme as PhoneLook;
}

/** What picking a look sets. Match device pairs Studio (dark) with Sleeve (light). */
export function phoneLookPatch(v: PhoneLook, current: Pick<Settings, "theme">): Partial<Settings> {
  if (v === "device") return { theme: current.theme === "studioBlack" ? "studioBlack" : "studio", followSystem: true, themeFile: null };
  if (v === "other") return {};
  return { theme: v, followSystem: false, themeFile: null };
}

export function phoneLookOptions(current: PhoneLook): SelectOption<PhoneLook>[] {
  const base: SelectOption<PhoneLook>[] = [
    { value: "device", label: "Match device" },
    { value: "studio", label: "Studio" },
    { value: "studioBlack", label: "Studio Black" },
    { value: "sleeve", label: "Sleeve" },
  ];
  return current === "other" ? [...base, { value: "other", label: "Another look" }] : base;
}

export function phoneRows(s: Settings | null): RowDef[] {
  return [
    {
      key: "phoneLook",
      section: "Appearance",
      title: "Look",
      description: "Match device follows the phone's light or dark: Studio when dark, Sleeve when light.",
      control: (ids) => {
        if (!s) return wait;
        const cur = phoneLookOf(s);
        return <Select value={cur} options={phoneLookOptions(cur)} onChange={(v) => void updateSettings(phoneLookPatch(v, s))} labelledBy={ids.title} describedBy={ids.desc} />;
      },
    },
    {
      key: "phoneMobileDownloads",
      section: "Downloads and devices",
      group: GROUP,
      title: "Download over mobile data",
      description: "Downloads also run away from Wi-Fi. This device only.",
      more: "Off: downloads wait for Wi-Fi. On: they also use mobile data, which may count against your plan. This device only.",
      control: (ids) => sw(s, "phoneMobileDownloads", ids),
    },
    {
      key: "phoneKeepRecent",
      section: "Downloads and devices",
      group: GROUP,
      title: "Keep recently played offline",
      description: "The last songs you played stay playable without a connection. This device only.",
      control: (ids) =>
        s ? (
          <Select
            value={String(s.phoneKeepRecent)}
            options={KEEP_RECENT}
            onChange={(v) => updateSettings({ phoneKeepRecent: Number(v) })}
            labelledBy={ids.title}
            describedBy={ids.desc}
          />
        ) : (
          wait
        ),
    },
    {
      key: "phoneMobileQuality",
      section: "Downloads and devices",
      group: GROUP,
      title: "Stream quality on mobile data",
      description: "Automatic keeps lossless files under 1.5 Mbps, else 320 kbps. This device only.",
      control: (ids) =>
        s ? (
          <Select value={s.phoneMobileQuality} options={MOBILE_QUALITY} onChange={(v) => updateSettings({ phoneMobileQuality: v })} labelledBy={ids.title} describedBy={ids.desc} />
        ) : (
          wait
        ),
    },
    {
      key: "phoneHaptics",
      section: "Downloads and devices",
      group: GROUP,
      title: "Haptics on controls",
      description: "A light tap when you press play, skip or switch tabs. This device only.",
      control: (ids) => sw(s, "phoneHaptics", ids),
    },
    {
      key: "carplayResume",
      section: "Downloads and devices",
      group: CARPLAY,
      title: "Resume when the car connects",
      description: "Plays on where you left off when CarPlay starts. This device only.",
      more: "Only what was playing plays on: nothing new starts, and a car that drops for a moment changes nothing. This device only.",
      control: (ids) => sw(s, "carplayResume", ids),
    },
    {
      key: "carplayOfflineDownloads",
      section: "Downloads and devices",
      group: CARPLAY,
      title: "Show downloads only when offline",
      description: "With no signal, CarPlay lists only what's downloaded. This device only.",
      more: "Off: everything stays listed, and what needs a connection says so. This device only.",
      control: (ids) => sw(s, "carplayOfflineDownloads", ids),
    },
    {
      key: "siriPhrases",
      section: "Downloads and devices",
      group: SIRI,
      title: "Ask Siri",
      description: 'Say "Hey Siri, play Abbey Road on Platter". Tap ? for everything you can say.',
      more: SIRI_PHRASES,
      control: () => null,
    },
    {
      key: "siriDislikeHides",
      section: "Downloads and devices",
      group: SIRI,
      title: "When you tell Siri you don't like a song",
      description: "Hidden songs aren't picked for mixes, radio or shuffle. Synced.",
      more: "Hide it and skip: the song is skipped and kept out of what Platter picks for you (Settings lists hidden songs). Just skip: it's only skipped. Synced.",
      control: (ids) =>
        s ? (
          <Select
            value={s.siriDislikeHides ? "hide" : "skip"}
            options={SIRI_DISLIKE}
            onChange={(v) => updateSettings({ siriDislikeHides: v === "hide" })}
            labelledBy={ids.title}
            describedBy={ids.desc}
          />
        ) : (
          wait
        ),
    },
  ];
}
