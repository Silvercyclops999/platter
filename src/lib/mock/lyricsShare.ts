/**
 * Mock of sharing a transcription to LRCLIB (docs/IPC.md "Sharing lyrics"):
 * the same offer rules as the core, and a fake publish that takes a moment.
 * Test hooks make the next publish fail (offline or rejected).
 */
import type { LyricsShareInfo, LyricsShareWhyNot, Settings, Track } from "../types";

export interface MockLyricsShareHooks {
  settings: () => Settings;
  track: (trackId: string) => Track | undefined;
  /** The song's own lyrics, with the model when they're a transcription. */
  own: (trackId: string) => { text: string; auto?: string } | undefined;
  unreleased: (track: Track) => boolean;
  later: (ms: number, fn: () => void) => void;
}

/** Transcribed (enhanced) LRC as LRCLIB takes it: line LRC and plain words. */
export function lrcForLrclib(text: string): { synced: string; plain: string } {
  const synced: string[] = [];
  const plain: string[] = [];
  for (const raw of text.split("\n")) {
    const line = raw.trim();
    if (!line) continue;
    const stamp = line.match(/^(\[[^\]]*\])+/)?.[0] ?? "";
    const words = line
      .slice(stamp.length)
      .replace(/<[^>]*>/g, "")
      .split(/\s+/)
      .filter(Boolean)
      .join(" ");
    if (!stamp) {
      if (words) plain.push(words);
      continue;
    }
    synced.push(stamp + words);
    if (words) plain.push(words);
    else if (plain.length && plain[plain.length - 1] !== "") plain.push("");
  }
  while (plain.length && plain[plain.length - 1] === "") plain.pop();
  return { synced: synced.join("\n"), plain: plain.join("\n") };
}

const WHY_NOT_TEXT: Record<LyricsShareWhyNot, string> = {
  off: "Sharing transcriptions is off in Settings.",
  notTranscribed: "Only lyrics transcribed on your computers can be shared.",
  notReviewed: "Check these lyrics first: open them in the lyrics editor or play the song through with them showing.",
  missingInfo: "This song needs a title, artist, album and length before its lyrics can be shared.",
  unreleased: "Sharing lyrics of unreleased songs is off in Settings.",
  shared: "These lyrics were already shared.",
};

export class MockLyricsShare {
  /** Songs whose lyrics the user reviewed. */
  reviewed = new Set<string>();
  /** Songs shared (on any device). */
  shared = new Set<string>();
  /** What was sent, in order. */
  sent: { trackId: string; payload: NonNullable<LyricsShareInfo["payload"]> }[] = [];
  /** Test hook: the next publish fails this way. */
  failNext: "offline" | "rejected" | null = null;
  /** How long a publish takes. */
  publishMs = 300;
  private running = new Map<string, () => void>();

  constructor(private hooks: MockLyricsShareHooks) {}

  reset() {
    this.reviewed.clear();
    this.shared.clear();
    this.sent = [];
    this.failNext = null;
    this.publishMs = 300;
    this.running.clear();
  }

  info(trackId: string): LyricsShareInfo {
    const s = this.hooks.settings();
    const t = this.hooks.track(trackId);
    const own = this.hooks.own(trackId);
    const unreleased = !!t && this.hooks.unreleased(t);
    const complete = !!t && !!t.title.trim() && !!(t.trackArtist || t.artistTitle).trim() && !!t.albumTitle.trim() && t.durationMs > 0;
    const whyNot: LyricsShareWhyNot | undefined = !s.shareTranscriptions
      ? "off"
      : this.shared.has(trackId)
        ? "shared"
        : !t || !own?.auto
          ? "notTranscribed"
          : unreleased && !s.shareTranscriptionsUnreleased
            ? "unreleased"
            : !complete
              ? "missingInfo"
              : !this.reviewed.has(trackId)
                ? "notReviewed"
                : undefined;
    if (whyNot || !t || !own) return { offer: false, whyNot, unreleased };
    const { synced, plain } = lrcForLrclib(own.text);
    return {
      offer: true,
      unreleased,
      payload: {
        trackName: t.title.trim(),
        artistName: (t.trackArtist || t.artistTitle).trim(),
        albumName: t.albumTitle.trim(),
        duration: Math.round(t.durationMs / 1000),
        plainLyrics: plain,
        syncedLyrics: synced,
      },
    };
  }

  markReviewed(trackId: string) {
    this.reviewed.add(trackId);
  }

  publish(trackId: string): Promise<void> {
    const i = this.info(trackId);
    if (!i.offer || !i.payload) return Promise.reject({ kind: "invalid", message: WHY_NOT_TEXT[i.whyNot ?? "notTranscribed"] });
    if (this.running.has(trackId)) return Promise.reject({ kind: "invalid", message: "These lyrics are already being shared." });
    if (this.hooks.settings().goOffline) return Promise.reject({ kind: "network", message: "Go offline is on. Turn it off to share lyrics." });
    const fail = this.failNext;
    this.failNext = null;
    const payload = i.payload;
    return new Promise((resolve, reject) => {
      this.running.set(trackId, () => reject({ kind: "invalid", message: "Sharing was cancelled. Nothing was sent." }));
      this.hooks.later(this.publishMs, () => {
        if (!this.running.delete(trackId)) return;
        if (fail === "offline")
          return reject({ kind: "network", message: "Couldn't reach LRCLIB. Check your connection and try again. Nothing was shared." });
        if (fail === "rejected") return reject({ kind: "invalid", message: "LRCLIB didn't take these lyrics: Lyrics are too short" });
        this.shared.add(trackId);
        this.sent.push({ trackId, payload });
        resolve();
      });
    });
  }

  cancel(trackId: string) {
    const stop = this.running.get(trackId);
    this.running.delete(trackId);
    stop?.();
  }
}
