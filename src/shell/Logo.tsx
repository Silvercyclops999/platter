/**
 * Platter mark: a P whose bowl is a record, a ring with the spindle in its
 * centre. The same drawing and grid as the app icon (src-tauri/app-icon.svg),
 * cropped to the mark. The P takes the look's accent, the spindle its text
 * colour. Below 20 px the spindle is left out, as it would only blur.
 */
const BOWL_AND_STEM = "M304 828V404A208 208 0 1 1 420 590.55V828Z M600 404A88 88 0 1 0 424 404A88 88 0 1 0 600 404Z";

export function Logo({ size = 28, label }: { size?: number; label?: string }) {
  const named = label ? { role: "img", "aria-label": label } : { "aria-hidden": true as const };
  return (
    <svg className="logo-mark" width={size} height={size} viewBox="196 196 632 632" focusable="false" {...named}>
      <path d={BOWL_AND_STEM} fill="var(--color-accent)" fillRule="evenodd" />
      {size >= 20 && <circle cx="512" cy="404" r="30" fill="var(--color-text-primary)" data-part="spindle" />}
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
