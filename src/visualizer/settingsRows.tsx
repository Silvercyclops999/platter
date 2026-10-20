/**
 * The visualizer's rows on the Settings page (Playback, under its own small
 * heading). Style, intensity and smoothing are synced; the frame rate, the
 * panel strip and resting on battery are this device only.
 */
import { Select, type SelectOption } from "../components/Select";
import { Skeleton } from "../components/Skeleton";
import { Switch } from "../components/Switch";
import type { Settings, VisualizerIntensity, VisualizerSmoothing, VisualizerStyle } from "../lib/types";
import type { RowDef } from "../pages/SettingsPage";
import { updateSettings } from "../stores/settings";
import { INTENSITY_CHOICES, STYLE_GROUPS, normalizeStyle } from "./styles";

type Ids = { title: string; desc: string };
type SwitchKey = "visualizerButton" | "visualizerPanelStrip" | "visualizerBatteryPause";

const GROUP = "Visualizer";
const wait = <Skeleton width={40} height={24} radius="pill" />;

/** The styles in their groups: Art first (Sleeve is the default), then Instruments. */
export const VIS_STYLE_CHOICES: SelectOption<VisualizerStyle>[] = STYLE_GROUPS.flatMap((g) => g.styles.map((st) => ({ value: st.id, label: st.label, group: g.name })));

export const VIS_INTENSITY_CHOICES: SelectOption<VisualizerIntensity>[] = INTENSITY_CHOICES.map((c) => ({ ...c }));

export const VIS_FPS_CHOICES: SelectOption<string>[] = [
  { value: "60", label: "60 frames a second" },
  { value: "30", label: "30 frames a second" },
  { value: "15", label: "15 frames a second" },
];

export const VIS_SMOOTHING_CHOICES: SelectOption<VisualizerSmoothing>[] = [
  { value: "low", label: "Low" },
  { value: "medium", label: "Medium" },
  { value: "high", label: "High" },
];

function sw(s: Settings | null, key: SwitchKey, ids: Ids) {
  if (!s) return wait;
  return <Switch checked={s[key]} onChange={(v) => void updateSettings({ [key]: v })} labelledBy={ids.title} describedBy={ids.desc} />;
}

function sel<T extends string>(s: Settings | null, value: T | undefined, options: SelectOption<T>[], ids: Ids, onChange: (v: T) => void) {
  if (!s || value === undefined) return <Skeleton width={120} height={32} radius="pill" />;
  return <Select value={value} options={options} onChange={onChange} labelledBy={ids.title} describedBy={ids.desc} />;
}

export function visualizerRows(s: Settings | null, phone: boolean): RowDef[] {
  const off = s && !s.visualizerButton ? "Turn on the Visualizer button to use this." : undefined;
  return [
    {
      key: "visualizerButton",
      section: "Playback",
      group: GROUP,
      title: "Show the Visualizer button",
      description: "A button in now playing that shows what the music is doing. V opens it too.",
      more: "A button in the expanded now playing view (and on the phone's now playing) that fills it with the cover and its colours moving with the music, or shows an instrument: a spectrum analyser, level meters or a stereo scope. It only runs while you can see it.",
      control: (ids) => sw(s, "visualizerButton", ids),
    },
    {
      key: "visualizerStyle",
      section: "Playback",
      group: GROUP,
      title: "Visualizer style",
      description: "Art over the cover (Sleeve, Ink, Grooves, Light) or an instrument. You can pick in now playing too.",
      more: "Art fills now playing with the cover and its colours, moving with the music. Sleeve: the cover's colours drift and swell with the bass. Ink: they flow like smoke, faster with the bass. Grooves: a record in the cover's colours, rippling on the beat. Light: soft light leaks and film grain that flare on hits. The instruments are an Analyser on a real frequency and level scale, a pair of needle Meters, and a stereo Scope. With Reduce motion on, Sleeve stays still and only Sleeve, Analyser and Meters run.",
      note: off,
      control: (ids) => sel(s, s ? normalizeStyle(s.visualizerStyle) : undefined, VIS_STYLE_CHOICES, ids, (v) => void updateSettings({ visualizerStyle: v })),
    },
    {
      key: "visualizerIntensity",
      section: "Playback",
      group: GROUP,
      title: "Visualizer intensity",
      description: "How much the art moves with the music: Calm or Lively.",
      more: "Calm moves slowly and swells gently with the beat, the kind of thing you leave on. Lively reacts more and quicker. It changes the art styles, not the instruments.",
      note: off,
      control: (ids) => sel(s, s?.visualizerIntensity, VIS_INTENSITY_CHOICES, ids, (v) => void updateSettings({ visualizerIntensity: v })),
    },
    {
      key: "visualizerFps",
      section: "Playback",
      group: GROUP,
      title: "Visualizer frame rate",
      description: phone ? "Fewer frames use less battery. This device only." : "Fewer frames use less of the processor. This device only.",
      more: "How many times a second the visualizer draws. A phone runs at most 30. With Reduce motion on it draws 15. This device only.",
      note: off,
      control: (ids) =>
        sel(s, s ? String(s.visualizerFps) : undefined, phone ? VIS_FPS_CHOICES.filter((c) => c.value !== "60") : VIS_FPS_CHOICES, ids, (v) =>
          void updateSettings({ visualizerFps: Number(v) }),
        ),
    },
    {
      key: "visualizerSmoothing",
      section: "Playback",
      group: GROUP,
      title: "Visualizer smoothing",
      description: "How slowly the lines settle after a loud moment.",
      note: off,
      control: (ids) => sel(s, s?.visualizerSmoothing, VIS_SMOOTHING_CHOICES, ids, (v) => void updateSettings({ visualizerSmoothing: v })),
    },
    {
      key: "visualizerPanelStrip",
      section: "Playback",
      group: GROUP,
      title: "Analyser under the cover in the now playing panel",
      description: "A slim spectrum line under the cover in the side panel. This device only.",
      control: (ids) => sw(s, "visualizerPanelStrip", ids),
    },
    {
      key: "visualizerBatteryPause",
      section: "Playback",
      group: GROUP,
      title: phone ? "Rest the visualizer in Low Power Mode" : "Rest the visualizer on battery",
      description: phone
        ? "When the phone is in Low Power Mode, the visualizer waits. This device only."
        : "On a laptop running on battery, the visualizer waits. This device only.",
      control: (ids) => sw(s, "visualizerBatteryPause", ids),
    },
  ];
}
