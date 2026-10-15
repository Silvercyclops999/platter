import { useEffect } from "react";
import { useCurrentEntry } from "../stores/router";
import { hideHints, usePowerHints } from "./powerKeys";
import "./power-hints.css";

/**
 * The jump hints of keyboard power mode: a two-letter label beside each
 * clickable thing in view. Scrolling, resizing, a click or a page change
 * hides them (the labels would no longer sit next to their targets).
 */
export function PowerHints() {
  const hints = usePowerHints((s) => s.hints);
  const typed = usePowerHints((s) => s.typed);
  const route = useCurrentEntry().route;
  useEffect(() => hideHints(), [route]);
  useEffect(() => {
    if (!hints) return;
    const hide = () => hideHints();
    window.addEventListener("scroll", hide, true);
    window.addEventListener("resize", hide);
    window.addEventListener("mousedown", hide, true);
    window.addEventListener("blur", hide);
    return () => {
      window.removeEventListener("scroll", hide, true);
      window.removeEventListener("resize", hide);
      window.removeEventListener("mousedown", hide, true);
      window.removeEventListener("blur", hide);
    };
  }, [hints]);
  if (!hints) return null;
  const shown = hints.filter((h) => h.label.startsWith(typed));
  return (
    <div className="power-hints" aria-hidden="true" data-testid="power-hints">
      {shown.map((h) => (
        <span key={h.label} className="power-hint" style={{ left: h.left, top: h.top }} data-hint={h.label}>
          <span className="power-hint__typed">{h.label.slice(0, typed.length)}</span>
          {h.label.slice(typed.length)}
        </span>
      ))}
    </div>
  );
}
