import { CircleHelp } from "lucide-react";
import { lazy, Suspense, useId, useState } from "react";
import { IconButton } from "../components/Button";
import { help } from "../stores/help";
import { useSettings } from "../stores/settings";
import "./help.css";

// The pages load the first time help shows, not at startup.
const InlineHelp = lazy(() => import("./InlineHelp"));

/**
 * The "?" next to a complex screen: opens its help page in the side panel,
 * next to the screen. Inside a dialog (which covers the panel) the section
 * opens in place instead. Settings, App, Show "?" help buttons hides them.
 */
export function HelpButton({ page, anchor, label, inline = false }: { page: string; anchor?: string; label: string; inline?: boolean }) {
  const on = useSettings((x) => x.settings?.showHelpButtons !== false);
  const [open, setOpen] = useState(false);
  const id = useId();
  if (!on) return null;
  if (!inline)
    return <IconButton className="help-button" size="sm" label={label} tooltip={label} icon={<CircleHelp size={16} />} onClick={() => help.open(page, anchor)} />;
  return (
    <>
      <IconButton
        className="help-button"
        size="sm"
        label={label}
        tooltip={label}
        aria-expanded={open}
        aria-controls={id}
        icon={<CircleHelp size={16} />}
        onClick={() => setOpen(!open)}
      />
      <div id={id} className="help-inline" hidden={!open}>
        {open && (
          <Suspense fallback={null}>
            <InlineHelp page={page} anchor={anchor} />
          </Suspense>
        )}
      </div>
    </>
  );
}

/** "Learn more" after a setting's description: always shown, it's part of the text. */
export function LearnMore({ page, anchor, about }: { page: string; anchor?: string; about: string }) {
  return (
    <button type="button" className="learn-more" aria-label={`Learn more about ${about}`} onClick={() => help.open(page, anchor)}>
      Learn more
    </button>
  );
}
