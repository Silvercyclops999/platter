import { X } from "lucide-react";
import { Button, IconButton } from "../components/Button";
import { framePatch, layoutOfferDue, newLookOfferDue, useLooks } from "../stores/looks";
import { updateSettings, useSettings } from "../stores/settings";
import { toast } from "../stores/toasts";
import "./new-look-offer.css";

/**
 * A one-time offer, for someone on Classic from before layouts, to try
 * Index. Never during the first-run step or while the new look offer is up,
 * and gone for good after either button (or the close button, which keeps
 * Classic).
 */
export function LayoutOffer() {
  const s = useSettings((st) => st.settings);
  const picking = useLooks((st) => st.pick);
  if (!s || picking || !layoutOfferDue(s) || newLookOfferDue(s)) return null;
  const keep = () => void updateSettings({ layoutOffered: true });
  const tryIt = () => {
    const old = s.layout;
    void updateSettings(framePatch("index", s));
    toast.undo("Switched to Index.", () => void updateSettings({ layout: old }));
  };
  return (
    <section className="new-look new-look--layout" role="region" aria-labelledby="layout-offer-title">
      <span className="new-look__mark new-look__mark--frame" aria-hidden="true">
        <span className="layout-glyph">
          <span className="layout-glyph__side" />
          <span className="layout-glyph__strip" />
          <span className="layout-glyph__page" />
        </span>
      </span>
      <div className="new-look__text">
        <h2 id="layout-offer-title" className="new-look__title">
          Try the Index layout
        </h2>
        <p className="new-look__line">Sections down the left, the player strip across the top and the queue in a drawer. Your look stays, and Classic is one click away under Appearance.</p>
        <div className="new-look__actions">
          <Button variant="primary" size="sm" onClick={tryIt}>
            Try Index
          </Button>
          <Button variant="secondary" size="sm" onClick={keep}>
            Keep Classic
          </Button>
        </div>
      </div>
      <IconButton className="new-look__close" label="Close, keep Classic" size="sm" icon={<X size={16} />} onClick={keep} />
    </section>
  );
}
