/**
 * The visualizer as now playing shows it (the expanded view, and the phone
 * scaled down): the chosen style, a row to pick another (remembered, and
 * synced), and plain words when there is nothing to draw.
 */
import { Suspense } from "react";
import type { VisualizerStyle } from "../lib/types";
import { usePlayer } from "../stores/player";
import { updateSettings, useSettings } from "../stores/settings";
import { Analyser } from "./Analyser";
import { useReducedMotion, useVisAccent } from "./draw";
import { useFeed, useSpectrumFeed } from "./feed";
import { Meters } from "./Meters";
import "./visualizer.css";

export const STYLES: readonly { id: VisualizerStyle; label: string }[] = [
  { id: "analyser", label: "Analyser" },
  { id: "meters", label: "Meters" },
  { id: "scope", label: "Scope" },
  { id: "glow", label: "Glow" },
];

/** The styles reduced motion keeps. */
export const CALM_STYLES: readonly VisualizerStyle[] = ["analyser", "meters"];
/** Frames a second under reduced motion. */
export const CALM_FPS = 15;
/** The phone's most. */
export const PHONE_MAX_FPS = 30;

/** The style to draw: the chosen one, unless reduced motion leaves it out. */
export function shownStyle(chosen: VisualizerStyle | undefined, reduced: boolean): VisualizerStyle {
  const s = chosen ?? "analyser";
  return reduced && !CALM_STYLES.includes(s) ? "analyser" : s;
}

/** Frames a second: the setting, capped on a phone, 15 under reduced motion. */
export function visFps(setting: number | undefined, phone: boolean, reduced: boolean): number {
  if (reduced) return CALM_FPS;
  const want = setting ?? (phone ? PHONE_MAX_FPS : 60);
  return phone ? Math.min(PHONE_MAX_FPS, want) : want;
}

/** What stops the picture, in words, or null when it can draw. */
export function useVisualizerBlock(): string | null {
  const remote = usePlayer((s) => s.remote !== null);
  const hasTrack = usePlayer((s) => !!s.state?.track);
  const resting = useFeed((s) => s.resting);
  if (remote) return "The visualizer shows what plays on this device.";
  if (!hasTrack) return "Nothing is playing on this device.";
  if (resting) return "Resting on battery power. You can change this in Settings, under Playback.";
  return null;
}

export function VisualizerView({ phone = false }: { phone?: boolean }) {
  const chosen = useSettings((s) => s.settings?.visualizerStyle);
  const smoothing = useSettings((s) => s.settings?.visualizerSmoothing ?? "medium");
  const fpsSetting = useSettings((s) => s.settings?.visualizerFps);
  const reduced = useReducedMotion();
  const style = shownStyle(chosen, reduced);
  const fps = visFps(fpsSetting, phone, reduced);
  const block = useVisualizerBlock();
  const remoteOrIdle = usePlayer((s) => s.remote !== null || !s.state?.track);
  const accent = useVisAccent("dark");
  useSpectrumFeed(remoteOrIdle ? null : { fps, scope: style === "scope" });

  let picture;
  if (style === "meters") picture = <Meters fps={fps} accent={accent} />;
  else picture = <Analyser fps={fps} smoothing={smoothing} peakHold={!reduced} accent={accent} />;

  return (
    <section className={`vis ${phone ? "vis--phone" : ""} vis--${style}`} aria-label="Visualizer" data-testid="visualizer">
      <div className="vis__panel">
        {block ? (
          <p className="vis__block" role="status">
            {block}
          </p>
        ) : (
          <Suspense fallback={null}>{picture}</Suspense>
        )}
      </div>
      <div className="vis__styles" role="radiogroup" aria-label="Visualizer style">
        {STYLES.map((s) => {
          const off = reduced && !CALM_STYLES.includes(s.id);
          const on = s.id === style;
          return (
            <button
              key={s.id}
              type="button"
              role="radio"
              aria-checked={on}
              aria-disabled={off || undefined}
              title={off ? "Off while Reduce motion is on" : undefined}
              className={`vis__style ${on ? "is-on" : ""}`}
              onClick={() => {
                if (!off && s.id !== chosen) void updateSettings({ visualizerStyle: s.id });
              }}
            >
              {s.label}
            </button>
          );
        })}
      </div>
    </section>
  );
}
