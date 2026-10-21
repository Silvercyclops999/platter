/**
 * A tapped song on its way (docs/IPC.md `lidarr_tap`): a thin line in place
 * of the seek bar that fills as it comes, with the step in words beside it.
 * The desktop player bar and the phone's player both show it.
 */
import type { Tap } from "../lib/types";
import { dismissTap, gettingFill, gettingLabel } from "../stores/taps";
import "./getting-line.css";

export function GettingLine({ tap }: { tap: Tap }) {
  const label = gettingLabel(tap.phase, tap.progress);
  const fill = gettingFill(tap.phase, tap.progress);
  const stuck = tap.phase === "waiting" || tap.phase === "failed";
  return (
    <div className={`getting-line ${stuck ? "getting-line--stuck" : ""}`} role="status" aria-live="polite" aria-label={`Getting ${tap.title}: ${label}`}>
      <span className="getting-line__label">{label}</span>
      <span className="getting-line__track" aria-hidden="true">
        <span className="getting-line__fill" style={{ transform: `scaleX(${fill})` }} />
      </span>
      {tap.message && (
        <span className="getting-line__why truncate" title={tap.message}>
          {tap.message}
        </span>
      )}
      {stuck && (
        <button type="button" className="getting-line__close" onClick={dismissTap}>
          OK
        </button>
      )}
    </div>
  );
}
