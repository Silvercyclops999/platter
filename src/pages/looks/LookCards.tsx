/**
 * Ready-made looks as cards with a live picture of each, picked with one
 * click; and the "Pick a look" step offered on first run (skippable).
 */
import { Check } from "lucide-react";
import { useMemo, useState } from "react";
import { Button } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import { readyLooks, type ReadyLook } from "../../design/looks/readymade";
import { CLASSIC_LAYOUT, type Frame } from "../../design/themes";
import { applyReadyLook, isReadyLookInUse, looks, skipPatch, useLooks } from "../../stores/looks";
import { updateSettings, useSettings } from "../../stores/settings";
import { useTheme } from "../../stores/theme";
import { FrameCards } from "./LayoutCards";
import { PreviewFrame } from "./LookPreview";
import "./looks-ui.css";

/** Platter's look three ways; every other ready-made look waits behind More looks. */
const FRONT_LOOKS: readonly string[] = ["studio", "studioBlack", "sleeve"];

export function LookCards({ labelledBy }: { labelledBy?: string }) {
  const s = useSettings((st) => st.settings);
  const layout = s?.layout ?? CLASSIC_LAYOUT;
  const all = useMemo(() => readyLooks(), []);
  const front = useMemo(() => FRONT_LOOKS.map((id) => all.find((r) => r.id === id)).filter((r): r is ReadyLook => !!r), [all]);
  // A look in use that isn't one of the three keeps its card showing.
  const inUseElsewhere = all.some((r) => !FRONT_LOOKS.includes(r.id) && isReadyLookInUse(r, s));
  const [more, setMore] = useState(false);
  const list = more || inUseElsewhere ? [...front, ...all.filter((r) => !FRONT_LOOKS.includes(r.id))] : front;
  return (
    <>
      <div className="look-cards" role="radiogroup" aria-label={labelledBy ? undefined : "Ready-made looks"} aria-labelledby={labelledBy}>
        {list.map((r) => {
          const on = isReadyLookInUse(r, s);
          const own = r.source.kind === "file" ? r.source.look.layout : undefined;
          return (
            <button key={r.id} type="button" role="radio" aria-checked={on} aria-label={r.name} className={`look-card ${on ? "is-on" : ""}`} onClick={() => void applyReadyLook(r)}>
              <PreviewFrame card base={r.base} tokens={r.tokens} parts={{ style: r.style, fonts: r.fonts, layout: own ?? layout }} label={`${r.name} preview`} />
              <span className="look-card__name">
                {r.name}
                {on && <Check size={14} aria-hidden="true" />}
              </span>
            </button>
          );
        })}
      </div>
      {list.length < all.length && (
        <Button size="sm" variant="secondary" className="look-cards__more" onClick={() => setMore(true)}>
          More looks
        </Button>
      )}
    </>
  );
}

/** The layouts the first-run step shows up front; Classic and Console sit under More. */
export const FIRST_RUN_FRAMES: readonly Frame[] = ["index", "deck", "turntable", "gatefold"];
const MORE_FRAMES: readonly Frame[] = ["classic", "console"];

/** The first-run step: pick a layout and a ready-made look, make your own, or skip (Index in Studio). */
export function PickLookDialog() {
  const open = useLooks((s) => s.pick);
  const builderOn = useSettings((s) => s.settings?.showLookBuilder !== false);
  const [more, setMore] = useState(false);
  const skip = () => {
    void updateSettings(skipPatch(useSettings.getState().settings, useTheme.getState().systemDark));
    looks.closePick();
  };
  return (
    <Dialog open={open} onClose={() => looks.closePick()} label="Pick a layout and a look" className="pick-look">
      <div className="pick-look__body">
        <h2 className="pick-look__title">Pick a layout and a look</h2>
        <p className="pick-look__lead">The layout places things and the look paints them. You can change either any time in Settings, under Appearance.</p>
        <section className="pick-look__section">
          <h3 className="pick-look__heading" id="pick-layout-title">
            Layout
          </h3>
          <FrameCards frames={more ? [...FIRST_RUN_FRAMES, ...MORE_FRAMES] : FIRST_RUN_FRAMES} labelledBy="pick-layout-title" />
          {!more && (
            <Button size="sm" variant="secondary" className="pick-look__more" onClick={() => setMore(true)}>
              More layouts
            </Button>
          )}
        </section>
        <section className="pick-look__section">
          <h3 className="pick-look__heading" id="pick-look-title">
            Look
          </h3>
          <LookCards labelledBy="pick-look-title" />
        </section>
        <div className="pick-look__actions">
          <Button variant="secondary" onClick={skip}>
            Skip
          </Button>
          {builderOn && (
            <Button
              variant="secondary"
              onClick={() => {
                looks.closePick();
                looks.openBuilder("colours");
              }}
            >
              Make your own
            </Button>
          )}
          <Button variant="primary" onClick={() => looks.closePick()}>
            Done
          </Button>
        </div>
      </div>
    </Dialog>
  );
}
