import type { ReactNode } from "react";
import { isHiRes, qualityLabel } from "../lib/format";
import { useSettings } from "../stores/settings";
import type { Track } from "../lib/types";

export interface BadgeProps {
  tone?: "neutral" | "quality" | "hires" | "version" | "accent" | "shelf";
  children: ReactNode;
  title?: string;
}

export function Badge({ tone = "neutral", children, title }: BadgeProps) {
  return (
    <span className={`badge badge--${tone}`} title={title}>
      {children}
    </span>
  );
}

/** "FLAC 24/96" style badge from a track's media info (hidden when the
 *  Show format badge setting is off). */
export function QualityBadge({ track }: { track: Pick<Track, "codec" | "bitDepth" | "sampleRate" | "bitrate"> }) {
  const show = useSettings((s) => s.settings?.showFormatBadge !== false);
  const label = qualityLabel(track);
  if (!label || !show) return null;
  return (
    <Badge tone={isHiRes(track) ? "hires" : "quality"} title={`Audio quality: ${label}`}>
      {label}
    </Badge>
  );
}
