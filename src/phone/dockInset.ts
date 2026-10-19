/**
 * How much of the bottom the dock covers: the tab bar, the mini player
 * while it shows, and the home indicator's safe area. The shell measures
 * the dock itself and sets it on the phone as --ph-dock-h, so every page's
 * last row scrolls fully above the mini player whatever the real sizes are
 * (larger text, a different safe area). phone.css works out a stand-in
 * from the fixed sizes until the first measure.
 */

export const DOCK_VAR = "--ph-dock-h";

/** The value for a measured dock height; null leaves the CSS stand-in. */
export function dockInset(height: number): string | null {
  return Number.isFinite(height) && height > 0 ? `${Math.ceil(height)}px` : null;
}

/** Measure `dock` now and whenever it changes size, onto `root`. Returns a stop. */
export function followDock(root: HTMLElement, dock: HTMLElement): () => void {
  const measure = () => {
    const v = dockInset(dock.getBoundingClientRect().height);
    if (v) root.style.setProperty(DOCK_VAR, v);
    else root.style.removeProperty(DOCK_VAR);
  };
  measure();
  const ro = typeof ResizeObserver !== "undefined" ? new ResizeObserver(measure) : null;
  ro?.observe(dock);
  window.addEventListener("resize", measure);
  return () => {
    ro?.disconnect();
    window.removeEventListener("resize", measure);
  };
}
