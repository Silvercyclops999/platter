/**
 * Badges from tags read off the files: the explicit "E", version badges
 * (Leak, Demo...) and an archive quality badge (Lossless, CDQ, LQ) when it
 * says something the format badge doesn't.
 */
import type { Track } from "../lib/types";
import { useTrackBadges } from "../stores/fileTags";
import { Badge } from "./Badge";
import "./file-tags.css";

const VERSION_LABEL: Record<string, string> = {
  unreleased: "Unreleased",
  leak: "Leak",
  demo: "Demo",
  snippet: "Snippet",
  alternate: "Alternate",
  session: "Session",
  remix: "Remix",
  live: "Live",
  build: "Build",
};

/** Badge text for an ARCHIVE_VERSION value; null for "released" and unknown values. */
export function versionLabel(version: string | null | undefined): string | null {
  return (version && VERSION_LABEL[version.trim().toLowerCase()]) || null;
}

const QUALITY_LABEL: Record<string, string> = { lossless: "Lossless", cdq: "CDQ", lq: "LQ" };

const LOSSLESS_CODECS = ["flac", "alac", "wav", "aiff", "pcm", "ape", "wavpack"];

/** What the file itself is: lossless, CD-quality lossy (256 kbps and up) or low quality. */
export function fileQualityClass(t: Pick<Track, "codec" | "bitrate">): "lossless" | "cdq" | "lq" | null {
  const codec = (t.codec ?? "").toLowerCase();
  if (!codec) return null;
  if (LOSSLESS_CODECS.includes(codec)) return "lossless";
  if (t.bitrate == null) return null;
  return t.bitrate >= 256 ? "cdq" : "lq";
}

/** The archive quality badge text, only when it differs from what the file is. */
export function archiveQualityLabel(quality: string | null | undefined, t: Pick<Track, "codec" | "bitrate">): string | null {
  const q = quality?.trim().toLowerCase();
  if (!q || !QUALITY_LABEL[q]) return null;
  return fileQualityClass(t) === q ? null : QUALITY_LABEL[q];
}

/** The small "E". */
export function ExplicitBadge() {
  return (
    <span className="explicit-badge" title="Explicit" aria-label="Explicit" role="img">
      E
    </span>
  );
}

/** "E" for a song whose file is tagged explicit. */
export function ExplicitMark({ trackId }: { trackId: string }) {
  const b = useTrackBadges(trackId);
  return b.explicit ? <ExplicitBadge /> : null;
}

/** The version badge for a song ("released" shows nothing). */
export function VersionBadge({ version }: { version: string | null | undefined }) {
  const label = versionLabel(version);
  return label ? (
    <Badge tone="version" title={`Version: ${label}`}>
      {label}
    </Badge>
  ) : null;
}

/** Source quality, after the version badge in the title line (the format column is too narrow for both). */
export function ArchiveQualityBadge({ track }: { track: Track }) {
  const b = useTrackBadges(track.id);
  const label = archiveQualityLabel(b.quality, track);
  return label ? (
    <Badge tone="neutral" title={`Source quality: ${label}`}>
      {label}
    </Badge>
  ) : null;
}
