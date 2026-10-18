/**
 * The phone's own settings rows (Downloads and devices, "On this phone").
 * They show only when Platter runs as a phone; every one is this device only.
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

function sw(s: Settings | null, key: "phoneMobileDownloads" | "phoneHaptics", ids: Ids) {
  if (!s) return wait;
  return <Switch checked={s[key]} onChange={(v) => updateSettings({ [key]: v })} labelledBy={ids.title} describedBy={ids.desc} />;
}

const GROUP = "On this phone";

export function phoneRows(s: Settings | null): RowDef[] {
  return [
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
  ];
}
