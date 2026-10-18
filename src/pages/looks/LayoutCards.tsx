/**
 * Layouts: frame cards with a live picture of each (in the look in use),
 * the frame's own options, and the region editor for a custom frame. Used by
 * Settings > Appearance > Layout and the first-run "Pick a layout and a
 * look" step. Pictures are made-up sample music, never the library.
 */
import { Check } from "lucide-react";
import { useId, useState, type ReactNode } from "react";
import { Button } from "../../components/Button";
import {
  CLASSIC_LAYOUT,
  DEFAULT_REGIONS,
  FRAME_COMPONENTS,
  REGION_NAMES,
  SLOTS,
  regionsProblem,
  type Frame,
  type FrameComponent,
  type Layout,
  type RegionName,
  type RegionSize,
  type Regions,
} from "../../design/themes/validate";
import type { Settings } from "../../lib/types";
import { framePatch } from "../../stores/looks";
import { updateSettings, useSettings } from "../../stores/settings";
import { lookPartsFrom, useTheme } from "../../stores/theme";
import { COLLAPSE_ORDER, COMPONENT_NAME, FRAME_INFO, REGION_LABEL, frameInfo } from "../../shell/frames";
import { PreviewFrame } from "./LookPreview";
import "./layouts-ui.css";

const layoutOf = (s: Settings | null): Layout => ({ ...structuredClone(CLASSIC_LAYOUT), ...(s?.layout ?? {}) });

/** One frame as a card: its picture, name and line. Frames still to come can't be picked. */
function FrameCard({ frame, on, onPick }: { frame: Frame; on: boolean; onPick: (f: Frame) => void }) {
  const s = useSettings((st) => st.settings);
  const resolved = useTheme((st) => st.resolved);
  const info = frameInfo(frame);
  const parts = lookPartsFrom(s);
  const layout = { ...layoutOf(s), frame };
  return (
    <button
      type="button"
      role="radio"
      aria-checked={on}
      aria-disabled={!info.ready || undefined}
      aria-label={info.ready ? info.name : `${info.name}, coming soon`}
      className={`frame-card ${on ? "is-on" : ""} ${info.ready ? "" : "is-soon"}`}
      onClick={() => info.ready && onPick(frame)}
    >
      <PreviewFrame base={resolved.base} tokens={resolved.tokens} parts={{ ...parts, layout }} label={`${info.name} preview`} />
      <span className="frame-card__name">
        {info.name}
        {on && <Check size={14} aria-hidden="true" />}
        {!info.ready && <span className="frame-card__soon">Coming soon</span>}
      </span>
      <span className="frame-card__line">{info.line}</span>
    </button>
  );
}

/** Frame cards as a radio group. Picking one switches to it at once. */
export function FrameCards({ frames, labelledBy }: { frames: readonly Frame[]; labelledBy?: string }) {
  const s = useSettings((st) => st.settings);
  const current = s?.layout?.frame ?? "classic";
  return (
    <div className="frame-cards" role="radiogroup" aria-label={labelledBy ? undefined : "Layouts"} aria-labelledby={labelledBy}>
      {frames.map((f) => (
        <FrameCard key={f} frame={f} on={current === f} onPick={(x) => void updateSettings(framePatch(x, useSettings.getState().settings))} />
      ))}
    </div>
  );
}

/** A small set of choices as pressed buttons. */
function Choice<T extends string>({ label, value, options, onChange }: { label: string; value: T; options: { value: T; label: string }[]; onChange: (v: T) => void }) {
  const id = useId();
  return (
    <div className="layout-choice">
      <span id={id} className="layout-choice__label">
        {label}
      </span>
      <div className="layout-choice__options" role="radiogroup" aria-labelledby={id}>
        {options.map((o) => (
          <button key={o.value} type="button" role="radio" aria-checked={value === o.value} className={`layout-choice__option ${value === o.value ? "is-on" : ""}`} onClick={() => onChange(o.value)}>
            {o.label}
          </button>
        ))}
      </div>
    </div>
  );
}

const SIZE_OPTIONS: { value: RegionSize; label: string }[] = [
  { value: "small", label: "Narrow" },
  { value: "medium", label: "Medium" },
  { value: "large", label: "Wide" },
];

/** Move a component to a region (or take it out), as a new arrangement. */
export function placeComponent(r: Regions, c: FrameComponent, to: RegionName | null): Regions {
  const next = Object.fromEntries(REGION_NAMES.map((n) => [n, r[n].filter((x) => x !== c)])) as Regions;
  if (to) next[to] = [...next[to], c];
  return next;
}

/**
 * The custom frame's editor: each component and the region it goes in,
 * from the regions the slot table allows. A change that would break a rule
 * (taking away the queue, say) isn't saved; the reason shows instead.
 */
export function RegionEditor({ layout }: { layout: Layout }) {
  const [problem, setProblem] = useState<string | null>(null);
  const r = layout.regions ?? DEFAULT_REGIONS;
  const put = (c: FrameComponent, to: RegionName | null) => {
    const next = placeComponent(r, c, to);
    const why = regionsProblem(next);
    setProblem(why);
    if (!why) void updateSettings({ layout: { ...layout, regions: next } });
  };
  const where = (c: FrameComponent): RegionName | "" => REGION_NAMES.find((n) => r[n].includes(c)) ?? "";
  return (
    <div className="region-editor">
      <p className="region-editor__lead">Choose where each part goes. The page always holds your library, and some parts only fit in some places.</p>
      <ul className="region-editor__list">
        {FRAME_COMPONENTS.filter((c) => c !== "library").map((c) => {
          const id = `region-${c}`;
          return (
            <li key={c} className="region-editor__row">
              <label htmlFor={id} className="region-editor__name">
                {COMPONENT_NAME[c]}
              </label>
              <select id={id} className="select region-editor__select" value={where(c)} onChange={(e) => put(c, (e.target.value || null) as RegionName | null)}>
                <option value="">Not shown</option>
                {SLOTS[c].map((n) => (
                  <option key={n} value={n}>
                    {REGION_LABEL[n]}
                  </option>
                ))}
              </select>
            </li>
          );
        })}
      </ul>
      {problem && (
        <p className="region-editor__problem" role="alert">
          {problem}
        </p>
      )}
      <div className="region-editor__sizes">
        <Choice label="Left side" value={layout.leftSize ?? "medium"} options={SIZE_OPTIONS} onChange={(v) => void updateSettings({ layout: { ...layout, leftSize: v } })} />
        <Choice label="Right side" value={layout.rightSize ?? "medium"} options={SIZE_OPTIONS} onChange={(v) => void updateSettings({ layout: { ...layout, rightSize: v } })} />
      </div>
    </div>
  );
}

/** Settings > Appearance > Layout: the cards, the frame's options, and the ways back. */
export function LayoutSettings({ extra }: { extra?: ReactNode }) {
  const s = useSettings((st) => st.settings);
  const layout = layoutOf(s);
  const frame = layout.frame;
  const isDefault = JSON.stringify(layout) === JSON.stringify({ ...structuredClone(CLASSIC_LAYOUT), frame });
  return (
    <div className="layout-settings">
      <FrameCards frames={FRAME_INFO.map((f) => f.id)} />
      {frame === "index" && (
        <Choice
          label="Player strip"
          value={layout.transport}
          options={[
            { value: "top", label: "Top" },
            { value: "bottom", label: "Bottom" },
          ]}
          onChange={(v) => void updateSettings({ layout: { ...layout, transport: v } })}
        />
      )}
      {frame === "custom" && <RegionEditor layout={layout} />}
      {frame !== "classic" && <p className="layout-settings__note">In a narrow window: {COLLAPSE_ORDER.map((x, i) => (i ? x.toLowerCase() : x)).join(", then ")}.</p>}
      <div className="layout-settings__actions">
        {extra}
        <Button size="sm" disabled={!s || isDefault} onClick={() => void updateSettings({ layout: { ...structuredClone(CLASSIC_LAYOUT), frame } })}>
          Reset layout
        </Button>
        <Button size="sm" disabled={!s || frame === "classic"} onClick={() => void updateSettings(framePatch("classic", s))}>
          Switch to Classic
        </Button>
      </div>
    </div>
  );
}
