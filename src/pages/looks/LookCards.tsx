/**
 * Ready-made looks as cards with a live picture of each, picked with one
 * click; and the "Pick a look" step offered on first run (skippable).
 */
import { Check } from "lucide-react";
import { useMemo } from "react";
import { Button } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import { readyLooks } from "../../design/looks/readymade";
import { CLASSIC_LAYOUT } from "../../design/themes";
import { applyReadyLook, isReadyLookInUse, looks, useLooks } from "../../stores/looks";
import { useSettings } from "../../stores/settings";
import { PreviewFrame } from "./LookPreview";
import "./looks-ui.css";

export function LookCards({ labelledBy }: { labelledBy?: string }) {
  const s = useSettings((st) => st.settings);
  const layout = s?.layout ?? CLASSIC_LAYOUT;
  const list = useMemo(() => readyLooks(), []);
  return (
    <div className="look-cards" role="radiogroup" aria-label={labelledBy ? undefined : "Ready-made looks"} aria-labelledby={labelledBy}>
      {list.map((r) => {
        const on = isReadyLookInUse(r, s);
        const own = r.source.kind === "file" ? r.source.look.layout : undefined;
        return (
          <button key={r.id} type="button" role="radio" aria-checked={on} aria-label={r.name} className={`look-card ${on ? "is-on" : ""}`} onClick={() => void applyReadyLook(r)}>
            <PreviewFrame base={r.base} tokens={r.tokens} parts={{ style: r.style, fonts: r.fonts, layout: own ?? layout }} label={`${r.name} preview`} />
            <span className="look-card__name">
              {r.name}
              {on && <Check size={14} aria-hidden="true" />}
            </span>
          </button>
        );
      })}
    </div>
  );
}

/** The first-run step: pick a ready-made look, make your own, or skip. */
export function PickLookDialog() {
  const open = useLooks((s) => s.pick);
  const builderOn = useSettings((s) => s.settings?.showLookBuilder !== false);
  return (
    <Dialog open={open} onClose={() => looks.closePick()} label="Pick a look" className="pick-look">
      <div className="pick-look__body">
        <h2 className="pick-look__title" id="pick-look-title">
          Pick a look
        </h2>
        <p className="pick-look__lead">Choose how Platter looks. You can change it any time in Settings, under Appearance.</p>
        <LookCards labelledBy="pick-look-title" />
        <div className="pick-look__actions">
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
