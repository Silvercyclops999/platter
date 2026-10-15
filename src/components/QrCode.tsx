/** A QR code from the core: rows of "0" and "1" (dark), drawn as squares with a quiet zone. */
export function QrCode({ qr, label, className }: { qr: { size: number; rows: string[] }; label: string; className?: string }) {
  const q = 2;
  const n = qr.size + q * 2;
  return (
    <svg className={className} viewBox={`0 0 ${n} ${n}`} role="img" aria-label={label} shapeRendering="crispEdges">
      <rect width={n} height={n} fill="#fff" />
      {qr.rows.flatMap((row, y) =>
        [...row].map((c, x) => (c === "1" ? <rect key={`${x}-${y}`} x={x + q} y={y + q} width={1} height={1} fill="#000" /> : null)),
      )}
    </svg>
  );
}
