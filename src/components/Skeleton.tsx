import type { CSSProperties } from "react";

export interface SkeletonProps {
  width?: number | string;
  height?: number | string;
  radius?: "row" | "card" | "pill" | "circle";
  className?: string;
  style?: CSSProperties;
}

/** Shimmering placeholder block. Browse pages use these, never spinners. */
export function Skeleton({ width, height = 14, radius = "row", className = "", style }: SkeletonProps) {
  return (
    <span
      aria-hidden="true"
      className={`skeleton skeleton--${radius} ${className}`}
      style={{ width, height, ...style }}
    />
  );
}

/** Text-line skeletons with natural varying widths. */
export function SkeletonLines({ lines = 2, widths = ["70%", "45%"], gap = 8 }: { lines?: number; widths?: string[]; gap?: number }) {
  return (
    <span className="skeleton-lines" style={{ gap }} aria-hidden="true">
      {Array.from({ length: lines }, (_, i) => (
        <Skeleton key={i} width={widths[i % widths.length]} height={i === 0 ? 14 : 12} />
      ))}
    </span>
  );
}
