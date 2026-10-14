import { qualityLabel } from "../lib/format";
import type { ConnectionKind, Settings, StreamQuality, Track } from "../lib/types";

/** The relay never streams above this (same as the core). */
export const RELAY_MAX_KBPS = 128;

const kbps = (q: StreamQuality | undefined): number | null => (!q || q === "original" ? null : Number(q));

/**
 * What is actually streamed for `track`: the file as it is, or an MP3
 * transcode at the connection's cap. Mirrors the core's rule: home quality on
 * the local connection, away quality on remote access, and the relay capped
 * at 128 kbps. Files already at or under the cap play untouched.
 */
export function streamQuality(
  track: Pick<Track, "codec" | "bitDepth" | "sampleRate" | "bitrate">,
  connection: ConnectionKind | null | undefined,
  settings: Pick<Settings, "qualityHome" | "qualityAway"> | null,
): string | null {
  const home = kbps(settings?.qualityHome);
  const away = kbps(settings?.qualityAway);
  const cap = connection === "remote" ? away : connection === "relay" ? Math.min(away ?? RELAY_MAX_KBPS, RELAY_MAX_KBPS) : home;
  if (cap === null || (track.bitrate != null && track.bitrate <= cap)) return qualityLabel(track);
  return `MP3 ${cap}`;
}
