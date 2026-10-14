import { X } from "lucide-react";
import { useId, useRef } from "react";
import { IconButton } from "../components/Button";
import { Dialog } from "../components/Dialog";
import { useUi } from "../stores/ui";
import { SECTION_TITLES, shortcutKeys, shortcutLabel, shortcutsIn, type ShortcutSection } from "./shortcuts";
import "./shortcuts.css";

const SECTIONS: ShortcutSection[] = ["basic", "playback", "navigation", "layout"];

/** The keyboard shortcuts window: four sections of actions with their key caps. */
export function ShortcutsHelp() {
  const open = useUi((s) => s.shortcutsOpen);
  const close = () => useUi.getState().setShortcutsOpen(false);
  const id = useId();
  // Focus starts on the list, so arrows and Page Down scroll it.
  const body = useRef<HTMLDivElement>(null);
  return (
    <Dialog open={open} onClose={close} label="Keyboard shortcuts" className="shortcuts" initialFocus={body}>
      <header className="shortcuts__head">
        <div>
          <h2 id={`${id}-t`} className="shortcuts__title">
            Keyboard shortcuts
          </h2>
          <p className="shortcuts__sub">Press {shortcutLabel("help")} to open or close this list</p>
        </div>
        <IconButton label="Close" size="sm" icon={<X size={18} />} onClick={close} />
      </header>
      <div ref={body} className="shortcuts__body" tabIndex={0} aria-label="Shortcuts">
        {SECTIONS.map((section) => (
          <section key={section} className="shortcuts__section" aria-labelledby={`${id}-${section}`}>
            <h3 id={`${id}-${section}`} className="shortcuts__section-title">
              {SECTION_TITLES[section]}
            </h3>
            <dl className="shortcuts__list">
              {shortcutsIn(section).map((s) => (
                <div key={s.id} className="shortcuts__row" data-shortcut={s.id}>
                  <dt className="shortcuts__action">{s.label}</dt>
                  <dd className="shortcuts__keys">
                    {shortcutKeys(s).map((k, i) => (
                      <kbd key={i} className="keycap">
                        {k}
                      </kbd>
                    ))}
                  </dd>
                </div>
              ))}
            </dl>
          </section>
        ))}
      </div>
    </Dialog>
  );
}
