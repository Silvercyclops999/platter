/**
 * The visualizer as now playing shows it. The art styles fill the whole
 * view (ArtStage, placed by the phone's now playing and the expanded view);
 * the instruments sit in a panel here. Both have the same style picker,
 * in its two groups, remembered and synced, and plain words when there is
 * nothing to draw.
 */
import { lazy, Suspense, useId } from "react";
import type { VisualizerStyle } from "../lib/types";
import { usePlayer } from "../stores/player";
import { updateSettings, useSettings } from "../stores/settings";
import { Analyser } from "./Analyser";
import { useReducedMotion, useVisAccent } from "./draw";
import { useFeed, useSpectrumFeed } from "./feed";
import { Meters } from "./Meters";
import { CALM_STYLES, STYLE_GROUPS, normalizeStyle, shownStyle, visFps } from "./styles";
import "./visualizer.css";

export { CALM_FPS, CALM_STYLES, PHONE_MAX_FPS, STYLES, shownStyle, visFps } from "./styles";

const Scope = lazy(() => import("./Scope").then((m) => ({ default: m.Scope })));

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

/** The style that shows now: the saved one (Glow is Sleeve now), as reduced motion allows. */
export function useShownStyle(): VisualizerStyle {
  const chosen = useSettings((s) => s.settings?.visualizerStyle);
  const reduced = useReducedMotion();
  return shownStyle(chosen, reduced);
}

/** Plain words for what reduced motion leaves out. */
export const CALM_NOTE = "Ink, Grooves, Light and Scope are off while Reduce motion is on.";

/** The styles in their two groups, Art and Instruments: plain pressed buttons, each a tab stop. */
export function StylePicker({ className = "" }: { className?: string }) {
  const chosen = useSettings((s) => s.settings?.visualizerStyle);
  const reduced = useReducedMotion();
  const style = shownStyle(chosen, reduced);
  const saved = normalizeStyle(chosen);
  const calmId = useId();
  return (
    <div className={`vis__picker ${className}`}>
      <div className="vis__styles" role="group" aria-label="Visualizer style">
        {STYLE_GROUPS.map((g) => (
          <div key={g.name} className="vis__group" role="group" aria-label={g.name}>
            <span className="vis__group-name" aria-hidden="true">
              {g.name}
            </span>
            {g.styles.map((s) => {
              const off = reduced && !CALM_STYLES.includes(s.id);
              const on = s.id === style;
              return (
                <button
                  key={s.id}
                  type="button"
                  aria-pressed={on}
                  aria-disabled={off || undefined}
                  aria-describedby={off ? calmId : undefined}
                  className={`vis__style ${on ? "is-on" : ""}`}
                  onClick={() => {
                    if (!off && s.id !== saved) void updateSettings({ visualizerStyle: s.id });
                  }}
                >
                  {s.label}
                </button>
              );
            })}
          </div>
        ))}
      </div>
      {reduced && (
        <p id={calmId} className="vis__note">
          {CALM_NOTE}
        </p>
      )}
    </div>
  );
}

/** The instruments' panel: Analyser, Meters or Scope, over the picker. */
export function VisualizerView({ phone = false }: { phone?: boolean }) {
  const smoothing = useSettings((s) => s.settings?.visualizerSmoothing ?? "medium");
  const fpsSetting = useSettings((s) => s.settings?.visualizerFps);
  const reduced = useReducedMotion();
  const style = useShownStyle();
  const fps = visFps(fpsSetting, phone, reduced);
  const block = useVisualizerBlock();
  const remoteOrIdle = usePlayer((s) => s.remote !== null || !s.state?.track);
  const accent = useVisAccent("dark");
  useSpectrumFeed(remoteOrIdle ? null : { fps, scope: style === "scope" });

  let picture;
  if (style === "meters") picture = <Meters fps={fps} accent={accent} />;
  else if (style === "scope") picture = <Scope fps={fps} accent={accent} />;
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
      <StylePicker />
    </section>
  );
}
