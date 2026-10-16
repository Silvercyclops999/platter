import { useEffect, useState } from "react";
import { Button } from "../components/Button";
import { Dialog } from "../components/Dialog";
import { backend } from "../lib/backend";
import { guides } from "../stores/guides";
import { help } from "../stores/help";
import { useSettings } from "../stores/settings";
import { newSince, seenVersion, setSeenVersion, type Release } from "./whatsNew";
import "./help.css";

/** After an update: what's new, each with a link to its help page. Shown once per update. */
export function WhatsNewDialog() {
  const [shown, setShown] = useState<{ version: string; releases: Release[] } | null>(null);
  const guidesOn = useSettings((x) => x.settings?.showSetupGuides !== false);
  useEffect(() => {
    let alive = true;
    backend.appInfo().then(
      (info) => {
        if (!alive || !info?.version) return;
        const releases = newSince(seenVersion(), info.version);
        setSeenVersion(info.version);
        if (releases.length) setShown({ version: info.version, releases });
      },
      () => {},
    );
    return () => {
      alive = false;
    };
  }, []);
  const close = () => setShown(null);
  const items = shown?.releases.flatMap((r) => r.items) ?? [];
  return (
    <Dialog open={!!shown} onClose={close} label="What's new in Platter" className="confirm whats-new">
      <h2 className="confirm__title">What's new in Platter {shown?.version}</h2>
      <ul className="whats-new__list">
        {items.map((it) => (
          <li key={it.text}>
            <p className="whats-new__text">{it.text}</p>
            {(it.help || (guidesOn && it.guide)) && (
              <span className="whats-new__links">
                {it.help && (
                  <button
                    type="button"
                    className="learn-more"
                    onClick={() => {
                      close();
                      help.open(it.help![0], it.help![1]);
                    }}
                  >
                    Learn more
                  </button>
                )}
                {guidesOn && it.guide && (
                  <button
                    type="button"
                    className="learn-more"
                    onClick={() => {
                      close();
                      guides.open(it.guide!);
                    }}
                  >
                    Start the guide
                  </button>
                )}
              </span>
            )}
          </li>
        ))}
      </ul>
      <div className="confirm__actions">
        <Button variant="primary" onClick={close}>
          Got it
        </Button>
      </div>
    </Dialog>
  );
}
