import { switchingText, useSwitching } from "../stores/switching";
import { AvatarFace } from "./TopBar";

/**
 * Covers the whole window, in the current look, while the app restarts as
 * another Plex Home user: their picture and name, with a slow soft pulse
 * (still under reduced motion).
 */
export function SwitchingScreen() {
  const { shown, to } = useSwitching();
  if (!shown) return null;
  return (
    <div className="switching" role="status" aria-live="assertive" data-testid="switching-screen">
      <div className="switching__body">
        {to && (
          <span className="switching__ring">
            <AvatarFace path={to.avatar} size={96} className="switching__face" fallback={to.name.trim().charAt(0).toUpperCase()} />
          </span>
        )}
        <p className="switching__text">{switchingText(to)}</p>
        <p className="switching__hint">Loading {to ? "their" : "your"} music, likes and settings</p>
      </div>
    </div>
  );
}
