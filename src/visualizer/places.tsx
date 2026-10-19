/**
 * The visualizer's smaller places: the slim Analyser strip under the cover
 * in the now playing panel, and the Deck faceplate's needle meters. Each
 * runs only while it shows, and only for music playing on this device.
 */
import { useFullWindow } from "../stores/fullWindow";
import { usePlayer } from "../stores/player";
import { useSettings } from "../stores/settings";
import { Analyser } from "./Analyser";
import { useReducedMotion, useVisAccent } from "./draw";
import { useSpectrumFeed } from "./feed";
import { Meters } from "./Meters";
import { visFps } from "./VisualizerView";
import "./visualizer.css";

/** Music is playing here (not on another device, not nothing). */
function useLocalMusic(): boolean {
  return usePlayer((s) => s.remote === null && !!s.state?.track);
}

/** The panel's strip, when its setting is on and the panel isn't under the expanded view. */
export function PanelStrip() {
  const on = useSettings((s) => s.settings?.visualizerPanelStrip === true);
  const expanded = useFullWindow((s) => s.open);
  const local = useLocalMusic();
  return on && !expanded && local ? <PanelStripOn /> : null;
}

function PanelStripOn() {
  const reduced = useReducedMotion();
  const fps = visFps(useSettings((s) => s.settings?.visualizerFps), false, reduced);
  const smoothing = useSettings((s) => s.settings?.visualizerSmoothing ?? "medium");
  const accent = useVisAccent();
  useSpectrumFeed({ fps, scope: false });
  return (
    <div className="np-vis-strip" data-testid="panel-visualizer">
      <Analyser fps={fps} smoothing={smoothing} peakHold={false} slim accent={accent} />
    </div>
  );
}

/**
 * The visualizer counts as on while it is the expanded view's mode (shown
 * now, or the one it opens in next).
 */
export function useVisualizerOn(): boolean {
  const button = useSettings((s) => s.settings?.visualizerButton !== false);
  const mode = useFullWindow((s) => (s.open || !s.prefs.remember ? s.mode : s.prefs.mode));
  return button && mode === "visualizer";
}

/** Whether the Deck faceplate shows its meters now. */
export function useDeckMeters(): boolean {
  const on = useVisualizerOn();
  const local = useLocalMusic();
  return on && local;
}

/** The Deck's needle meters, in a recessed well on the faceplate, while the visualizer is on. */
export function DeckMeters() {
  return useDeckMeters() ? <DeckMetersOn /> : null;
}

function DeckMetersOn() {
  const reduced = useReducedMotion();
  const fps = visFps(useSettings((s) => s.settings?.visualizerFps), false, reduced);
  const accent = useVisAccent();
  useSpectrumFeed({ fps, scope: false });
  return (
    <div className="deck-meters" data-testid="deck-meters">
      <Meters fps={fps} accent={accent} compact />
    </div>
  );
}
