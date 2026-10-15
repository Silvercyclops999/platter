import type { Album, ReleaseSection, Track } from "./types";

/** 3:07, 1:02:45 */
export function formatDuration(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const ss = String(s).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${m}:${ss}`;
}

/** "1 hr 12 min", "38 min 20 sec" */
export function formatLongDuration(ms: number): string {
  const total = Math.round(ms / 1000);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  if (h > 0) return `${h} hr ${m} min`;
  return s && m < 10 ? `${m} min ${s} sec` : `${m} min`;
}

/** Quality badge text: "FLAC 24/96", "FLAC", "MP3 320", "AAC 256". */
export function qualityLabel(t: Pick<Track, "codec" | "bitDepth" | "sampleRate" | "bitrate">): string | null {
  if (!t.codec) return null;
  const codec = t.codec.toUpperCase();
  const lossless = ["FLAC", "ALAC", "WAV", "AIFF"].includes(codec);
  if (lossless) {
    if (t.bitDepth && t.sampleRate && (t.bitDepth > 16 || t.sampleRate > 48000)) {
      const khz = t.sampleRate / 1000;
      return `${codec} ${t.bitDepth}/${Number.isInteger(khz) ? khz : khz.toFixed(1)}`;
    }
    return codec;
  }
  return t.bitrate ? `${codec} ${t.bitrate}` : codec;
}

export function isHiRes(t: Pick<Track, "bitDepth" | "sampleRate">): boolean {
  return (t.bitDepth ?? 0) > 16 || (t.sampleRate ?? 0) > 48000;
}

export const SECTION_LABELS: Record<ReleaseSection, string> = {
  albums: "Albums",
  singlesEps: "Singles & EPs",
  live: "Live",
  compilations: "Compilations",
  soundtracks: "Soundtracks",
  remixes: "Remixes",
  demos: "Demos",
  mixtapes: "Mixtapes",
  unreleased: "Unreleased",
};

/** "Album", "EP", "Single", "Live album", "Soundtrack"… */
export function releaseTypeLabel(a: Pick<Album, "format" | "subformats">): string {
  const sub = a.subformats[0];
  if (sub === "live") return "Live album";
  if (sub === "soundtrack") return "Soundtrack";
  if (sub === "compilation") return "Compilation";
  if (sub === "remix") return "Remixes";
  if (sub === "demo") return "Demo";
  if (sub === "mixtape") return "Mixtape";
  if (a.format === "ep") return "EP";
  if (a.format === "single") return "Single";
  return "Album";
}

/** Disk size for people: "412 MB", "2 GB", "1.5 GB". */
export function formatBytes(n: number): string {
  const mb = n / (1024 * 1024);
  if (mb < 1) return n > 0 ? "less than 1 MB" : "0 MB";
  if (mb < 1024) return `${Math.round(mb)} MB`;
  const gb = mb / 1024;
  return `${gb >= 10 || Number.isInteger(gb) ? Math.round(gb) : gb.toFixed(1)} GB`;
}

export function plural(n: number, one: string, many = one + "s"): string {
  return `${n.toLocaleString()} ${n === 1 ? one : many}`;
}

export function trackArtistLine(t: Pick<Track, "trackArtist" | "artistTitle">): string {
  return t.trackArtist || t.artistTitle;
}
