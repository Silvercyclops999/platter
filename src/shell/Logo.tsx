/** Platter mark: a record platter seen from above, with an amber label. Original artwork. */
export function Logo({ size = 28 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 32 32" aria-hidden="true" focusable="false">
      <circle cx="16" cy="16" r="15" fill="#1C1916" stroke="#3A332C" strokeWidth="1" />
      <circle cx="16" cy="16" r="11.5" fill="none" stroke="#2C2722" strokeWidth="1" />
      <circle cx="16" cy="16" r="8.5" fill="none" stroke="#2C2722" strokeWidth="1" />
      <circle cx="16" cy="16" r="5.5" fill="#F5A524" />
      <circle cx="16" cy="16" r="1.3" fill="#1A1206" />
      <path d="M24.5 5.5 L19.5 13" stroke="#B8AFA4" strokeWidth="1.6" strokeLinecap="round" />
    </svg>
  );
}
