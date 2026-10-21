import { useEffect, useState } from "react";
import { Banner } from "../components/Banner";
import { Button } from "../components/Button";
import { Dialog } from "../components/Dialog";
import { backend } from "../lib/backend";
import { useApp } from "../stores/app";
import { toast } from "../stores/toasts";
import { useUi } from "../stores/ui";
import { healthBanner, type BannerAction } from "./bannerView";

export const ACTION_LABEL: Record<BannerAction, string> = {
  signIn: "Sign in",
  signOut: "Use another account",
  libraryGuide: "How to add one",
  localNetwork: "Open Settings",
};

export function LibraryGuide({ open, onClose }: { open: boolean; onClose: () => void }) {
  return (
    <Dialog open={open} onClose={onClose} label="Add a music library in Plex" className="guide">
      <h2 className="guide__title">Add a music library in Plex</h2>
      <ol className="guide__steps">
        <li>Open Plex on your server (in a browser, go to app.plex.tv and pick the server).</li>
        <li>Next to Libraries, choose More, then Add Library.</li>
        <li>Pick Music, add the folder that holds your music, then choose Add Library.</li>
        <li>Platter checks every minute and shows your music once Plex has scanned it.</li>
      </ol>
      <div className="guide__actions">
        <Button variant="primary" onClick={onClose}>
          Got it
        </Button>
      </div>
    </Dialog>
  );
}

/** Does what a banner's button says; the library guide is the caller's to open. */
export function runBannerAction(a: BannerAction, openGuide: () => void) {
  if (a === "signIn") useUi.getState().setSignInOpen(true);
  else if (a === "signOut") void backend.authSignOut().catch(() => {});
  else if (a === "localNetwork") void backend.openLocalNetworkSettings().catch(() => toast.error("Couldn't open System Settings"));
  else openGuide();
}

/** The app-state banner at the top of the main view. */
export function HealthBanner() {
  const health = useApp((s) => s.health);
  const [now, setNow] = useState(() => Date.now());
  const [guide, setGuide] = useState(false);
  const offline = health?.banner?.kind === "offline";

  // Keep "Last seen N min ago" current.
  useEffect(() => {
    if (!offline) return;
    setNow(Date.now());
    const t = setInterval(() => setNow(Date.now()), 30_000);
    return () => clearInterval(t);
  }, [offline]);

  const view = healthBanner(health, now);
  if (!view) return null;

  const run = (a: BannerAction) => runBannerAction(a, () => setGuide(true));

  return (
    <div className="main__banner">
      <Banner
        kind={view.kind}
        action={
          view.action && (
            <Button size="sm" variant="secondary" onClick={() => run(view.action!)}>
              {ACTION_LABEL[view.action]}
            </Button>
          )
        }
      >
        {view.message}
      </Banner>
      <LibraryGuide open={guide} onClose={() => setGuide(false)} />
    </div>
  );
}
