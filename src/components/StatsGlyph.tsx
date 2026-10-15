/**
 * Platter's Stats symbol: three level bars standing on a record label (the
 * ring is the label's edge, the dot its spindle hole). Drawn in
 * currentColor, so it takes whatever colour its place gives it.
 */
export function StatsGlyph({ size = 24, className = "" }: { size?: number | string; className?: string }) {
  return (
    <svg className={`stats-glyph ${className}`} width={size} height={size} viewBox="0 0 24 24" aria-hidden="true" focusable="false">
      <circle cx="12" cy="12" r="10" fill="none" stroke="currentColor" strokeWidth="1.8" />
      <rect x="6.9" y="10.4" width="2.7" height="6.2" rx="1.35" fill="currentColor" />
      <rect x="10.65" y="6.4" width="2.7" height="10.2" rx="1.35" fill="currentColor" />
      <rect x="14.4" y="8.6" width="2.7" height="8" rx="1.35" fill="currentColor" />
      <circle cx="12" cy="19.1" r="0.95" fill="currentColor" />
    </svg>
  );
}
