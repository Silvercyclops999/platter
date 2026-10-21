/**
 * The visualizer's state the views share: the style (remembered in the
 * synced settings, never a row of its own), and stepping through the styles
 * with a swipe or an arrow key.
 */
import type { VisualizerStyle } from "../lib/types";
import { updateSettings, useSettings } from "../stores/settings";
import { normalizeStyle, stepStyle } from "./styles";

/** The style to draw. */
export function useVisStyle(): VisualizerStyle {
  return useSettings((s) => normalizeStyle(s.settings?.visualizerStyle));
}

/** Whether a tap or click on the cover in now playing opens the visualizer. */
export function useCoverTap(): boolean {
  return useSettings((s) => s.settings?.visualizerCoverTap !== false);
}

/** The next (1) or previous (-1) style, remembered. */
export function stepVisStyle(step: 1 | -1) {
  const now = useSettings.getState().settings?.visualizerStyle;
  void updateSettings({ visualizerStyle: stepStyle(now, step) });
}
