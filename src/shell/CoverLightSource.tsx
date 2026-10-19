/**
 * Puts the playing cover's light on <html> as --cover-light, for the Studio
 * looks' now playing areas (instruments.css). Nothing playing, the setting
 * off or Artwork colours off: no light. It changes only when the song does.
 */
import { useEffect } from "react";
import { coverLightColour } from "../design/coverLight";
import { usePlayer } from "../stores/player";
import { useSettings } from "../stores/settings";
import { useThemeBase } from "../stores/theme";
import { useCoverColors } from "./nowPlaying";

export function CoverLightSource() {
  const thumb = usePlayer((s) => s.state?.track?.thumb ?? null);
  const on = useSettings((s) => (s.settings?.coverLight ?? "on") !== "off" && s.settings?.artworkColours !== false);
  const base = useThemeBase();
  const c = useCoverColors(on ? thumb : null);
  const light = c ? coverLightColour(c.vivid, base) : null;
  useEffect(() => {
    const root = document.documentElement;
    if (light) root.style.setProperty("--cover-light", light);
    else root.style.removeProperty("--cover-light");
  }, [light]);
  useEffect(
    () => () => {
      document.documentElement.style.removeProperty("--cover-light");
    },
    [],
  );
  return null;
}
