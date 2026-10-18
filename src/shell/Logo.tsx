/**
 * Platter mark: a platter seen at an angle with a tonearm, the label and
 * headshell in the accent colour. The same drawing as the app icon
 * (src-tauri/app-icon.svg), simplified for small sizes. Original artwork.
 */
export function Logo({ size = 28 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 32 32" aria-hidden="true" focusable="false">
      <ellipse cx="14" cy="19.4" rx="12.6" ry="7.6" fill="#141311" stroke="var(--color-text-secondary)" strokeWidth="1.3" />
      <ellipse cx="14" cy="19.4" rx="8" ry="4.7" fill="none" stroke="#3A3631" strokeWidth="0.8" />
      <ellipse cx="14" cy="19.4" rx="4" ry="2.4" fill="var(--color-accent)" />
      <circle cx="27" cy="5.6" r="2.3" fill="var(--color-text-secondary)" />
      <path d="M27 5.6 L24.4 14.6 L21.6 17.4" fill="none" stroke="var(--color-text-secondary)" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
      <path d="M21.6 17.4 L19.9 19.1" stroke="var(--color-accent)" strokeWidth="2.6" strokeLinecap="round" />
    </svg>
  );
}
