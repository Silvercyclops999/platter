/**
 * The visualizer's one row on the Settings page (Playback). The style is
 * remembered from now playing (a swipe or an arrow key changes it), so it
 * has no row of its own.
 */
import { Skeleton } from "../components/Skeleton";
import { Switch } from "../components/Switch";
import type { Settings } from "../lib/types";
import type { RowDef } from "../pages/SettingsPage";
import { updateSettings } from "../stores/settings";

export function visualizerRows(s: Settings | null, phone: boolean): RowDef[] {
  return [
    {
      key: "visualizerCoverTap",
      section: "Playback",
      title: phone ? "Open the visualizer when you tap the cover" : "Open the visualizer when you click the cover",
      description: phone ? "Its colours move to the music. Swipe sideways for another style." : "Its colours move to the music. The arrow keys change the style.",
      more: phone
        ? "Four styles: Grooves, a record in the cover's colours that ripples on the beat; Blur, the cover's colours as soft shapes that swell with the bass; Ring, a record with a ring of marks round it that grow with the music; and Cover, just the album art. Tap the cover again to close it. It only runs while you can see it."
        : "Four styles: Grooves, a record in the cover's colours that ripples on the beat; Blur, the cover's colours as soft shapes that swell with the bass; Ring, a record with a ring of marks round it that grow with the music; and Cover, just the album art. V or the Visualizer button in the expanded view opens it too, and a click on the cover or Escape closes it. It only runs while you can see it.",
      control: (ids) =>
        s ? (
          <Switch checked={s.visualizerCoverTap} onChange={(v) => void updateSettings({ visualizerCoverTap: v })} labelledBy={ids.title} describedBy={ids.desc} />
        ) : (
          <Skeleton width={40} height={24} radius="pill" />
        ),
    },
  ];
}
