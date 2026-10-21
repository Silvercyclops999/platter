/**
 * Full-screen views with their own background (Wrapped, full-screen lyrics,
 * now playing over art) hold the status bar's text to suit it while they
 * show, light over a dark background and dark over a light one. The newest
 * hold wins; when the last one lets go, the look's own style comes back
 * (followStatusBar in PhoneShell reads this).
 */

const holds: { light: boolean }[] = [];
const listeners = new Set<() => void>();

/** The text a showing full-screen view asks for: true light, false dark, null when none holds it. */
export const heldStatusBar = (): boolean | null => (holds.length ? holds[holds.length - 1].light : null);

/** Whether something dark full-screen is showing. */
export const lightTextHeld = () => heldStatusBar() === true;

/** Ask for light (or dark) status bar text until the returned function is called (once). */
export function holdStatusBar(light: boolean): () => void {
  const hold = { light };
  holds.push(hold);
  listeners.forEach((l) => l());
  let released = false;
  return () => {
    if (released) return;
    released = true;
    holds.splice(holds.indexOf(hold), 1);
    listeners.forEach((l) => l());
  };
}

/** Ask for light status bar text until the returned function is called (once). */
export const holdLightStatusBar = () => holdStatusBar(true);

/** Hear when a hold starts or ends. */
export function onLightHoldChange(cb: () => void): () => void {
  listeners.add(cb);
  return () => listeners.delete(cb);
}

/** Forget every hold (tests). */
export function resetStatusBarHolds() {
  holds.length = 0;
}

/** "#rrggbb" (or "rrggbb") as 0 to 1 channels, or null. */
function rgb(hex: string): [number, number, number] | null {
  const m = /^#?([0-9a-f]{6})$/i.exec(hex.trim());
  if (!m) return null;
  const n = parseInt(m[1], 16);
  return [(n >> 16) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255];
}

/**
 * Whether `share` of `colour` over `base` is dark enough that dark status
 * bar text would be harder to read than light (relative luminance under 0.18).
 */
export function mixIsDark(colour: string | null | undefined, share: number, base = "#ffffff"): boolean {
  const c = colour ? rgb(colour) : null;
  const b = rgb(base);
  if (!c || !b) return false;
  const lin = (v: number) => (v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4);
  const [r, g, bl] = c.map((v, i) => lin(v * share + b[i] * (1 - share)));
  // Under about 0.18 white text reads better than black (their contrast ratios cross there).
  return 0.2126 * r + 0.7152 * g + 0.0722 * bl < 0.18;
}
