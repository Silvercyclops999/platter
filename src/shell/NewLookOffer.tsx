import { X } from "lucide-react";
import { Button, IconButton } from "../components/Button";
import type { Settings } from "../lib/types";
import { NEW_LOOK_PATCH, newLookOfferDue, useLooks } from "../stores/looks";
import { updateSettings, useSettings } from "../stores/settings";
import { toast } from "../stores/toasts";
import { Logo } from "./Logo";
import "./new-look-offer.css";

/** The settings the new look changes, as they are now (for Undo). */
function before(s: Settings): Partial<Settings> {
  return { theme: s.theme, accent: s.accent, themeFile: s.themeFile, followSystem: s.followSystem, recordMotion: s.recordMotion, tonearmSeek: s.tonearmSeek };
}

/**
 * A one-time offer, for someone who used Platter before Studio, to try its
 * own look. Shown once settings load, never during the first-run look step,
 * and gone for good after either button.
 */
export function NewLookOffer() {
  const s = useSettings((st) => st.settings);
  const picking = useLooks((st) => st.pick);
  if (!s || picking || !s.lookStepDone || !newLookOfferDue(s)) return null;
  const dismiss = () => void updateSettings({ newLookOffered: true });
  const tryIt = () => {
    const old = before(s);
    void updateSettings(NEW_LOOK_PATCH);
    toast.undo("Switched to Studio.", () => void updateSettings(old));
  };
  return (
    <section className="new-look" role="region" aria-labelledby="new-look-title">
      <span className="new-look__mark" aria-hidden="true">
        <Logo size={36} />
      </span>
      <div className="new-look__text">
        <h2 id="new-look-title" className="new-look__title">
          Try Platter's new look
        </h2>
        <p className="new-look__line">Studio: warm graphite, a brass accent and a record player in the expanded view. Your layout stays, and you can switch back under Appearance.</p>
        <div className="new-look__actions">
          <Button variant="primary" size="sm" onClick={tryIt}>
            Try it
          </Button>
          <Button variant="secondary" size="sm" onClick={dismiss}>
            Not now
          </Button>
        </div>
      </div>
      <IconButton className="new-look__close" label="Close" size="sm" icon={<X size={16} />} onClick={dismiss} />
    </section>
  );
}
