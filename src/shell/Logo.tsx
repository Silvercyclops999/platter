/**
 * Platter mark: a P whose bowl is a record, a ring with the spindle in its
 * centre and the counter as the record's label. The same drawing as the app
 * icon (src-tauri/app-icon.svg), in a square box the width of the mark's
 * height, centred on the mark's outline (next to text the outline is what
 * lines up; the icon's weight offset is for the tile only). The P takes the
 * look's accent, the spindle its text colour. Below 20 px the spindle is left
 * out: at 16 px it would be a one-pixel dot in a three-pixel hole.
 */
export const MARK_VIEWBOX = "206 216 660 660";
export const BOWL_AND_STEM = "M296 876V456A240 240 0 1 1 408 659.02V876Z M616 456A80 80 0 1 0 456 456A80 80 0 1 0 616 456Z";
export const SPINDLE = { cx: 536, cy: 456, r: 32 };

export function Logo({ size = 28, label }: { size?: number; label?: string }) {
  const named = label ? { role: "img", "aria-label": label } : { "aria-hidden": true as const };
  return (
    <svg className="logo-mark" width={size} height={size} viewBox={MARK_VIEWBOX} focusable="false" {...named}>
      <path d={BOWL_AND_STEM} fill="var(--color-accent)" fillRule="evenodd" />
      {size >= 20 && <circle cx={SPINDLE.cx} cy={SPINDLE.cy} r={SPINDLE.r} fill="var(--color-text-primary)" data-part="spindle" />}
    </svg>
  );
}

/** The mark with the name set in the display serif, for sign-in and welcome screens. */
export function Wordmark({ size = 26, className = "" }: { size?: number; className?: string }) {
  return (
    <span className={`wordmark ${className}`.trim()} role="img" aria-label="Platter">
      <Logo size={size} />
      <span className="wordmark__name" aria-hidden="true">
        Platter
      </span>
    </span>
  );
}
