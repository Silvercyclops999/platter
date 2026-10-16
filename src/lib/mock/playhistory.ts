import type { PlayHistorySource, PlayHistorySourceStatus, PlayHistoryStatus, Settings, TautulliStatus } from "../types";

const DAY = 86_400_000;

/**
 * Play history from Plex, Tautulli and Last.fm in the browser mock. The demo
 * has its Plex history already brought in; Tautulli is set up once its
 * address and key are saved, Last.fm once you connect it for scrobbling.
 */
type Row = { plays: number; notInLibrary: number; shortPlays: number; updatedAt: number | null; removed: boolean; error: string | null };

const fresh = (): Record<PlayHistorySource, Row> => ({
  plexHistory: { plays: 12_480, notInLibrary: 312, shortPlays: 0, updatedAt: Date.now() - DAY, removed: false, error: null },
  tautulli: { plays: 0, notInLibrary: 0, shortPlays: 0, updatedAt: null, removed: false, error: null },
  lastfm: { plays: 0, notInLibrary: 0, shortPlays: 0, updatedAt: null, removed: false, error: null },
});

export class MockPlayHistory {
  private hasKey = false;
  private rows = fresh();
  /** Failures the next updates meet, per source (tests and ?playHistory=). */
  private failing: Partial<Record<PlayHistorySource, string>> = {};

  constructor(
    private settings: () => Settings,
    private lastfmConnected: () => boolean,
  ) {}

  reset() {
    this.hasKey = false;
    this.rows = fresh();
    this.failing = {};
  }

  /** Make a source's updates fail with this message (null: work again). */
  fail(source: PlayHistorySource, message: string | null) {
    if (message) {
      this.failing[source] = message;
      this.rows[source].error = message;
    } else delete this.failing[source];
  }

  tautulli(): TautulliStatus {
    return { urlSet: !!this.settings().tautulliUrl.trim(), hasKey: this.hasKey };
  }

  setKey(key: string | null): TautulliStatus {
    const k = key?.trim() ?? "";
    if (k && !/^[A-Za-z0-9]{1,200}$/.test(k)) {
      throw { kind: "invalid", message: "A Tautulli API key is letters and numbers, from Tautulli's Settings > Web Interface." };
    }
    this.hasKey = !!k;
    return this.tautulli();
  }

  status(): PlayHistoryStatus {
    const s = this.settings();
    const t = this.tautulli();
    const setUp: Record<PlayHistorySource, boolean> = { plexHistory: true, tautulli: t.urlSet && t.hasKey, lastfm: this.lastfmConnected() };
    const included: Record<PlayHistorySource, boolean> = {
      plexHistory: s.includePlexHistory,
      tautulli: s.includeTautulliHistory,
      lastfm: s.includeLastfmHistory,
    };
    const sources = (["plexHistory", "tautulli", "lastfm"] as const).map(
      (source): PlayHistorySourceStatus => ({
        source,
        setUp: setUp[source],
        included: included[source],
        ...this.rows[source],
        firstReadDone: this.rows[source].updatedAt !== null,
        importing: false,
        done: 0,
        total: null,
        paused: false,
      }),
    );
    return { sources };
  }

  /** An update finds nothing new in the demo, but brings a newly set-up source in. */
  update(): PlayHistoryStatus {
    for (const st of this.status().sources) {
      const r = this.rows[st.source];
      if (!st.setUp || !st.included || r.removed) continue;
      const failing = this.failing[st.source];
      r.error = failing ?? null;
      if (failing) continue;
      if (r.updatedAt === null && st.source !== "plexHistory") {
        r.plays = st.source === "lastfm" ? 8_210 : 4_966;
        r.notInLibrary = st.source === "lastfm" ? 1_204 : 0;
        r.shortPlays = st.source === "tautulli" ? 311 : 0;
      }
      r.updatedAt = Date.now();
    }
    return this.status();
  }

  remove(source: PlayHistorySource): PlayHistoryStatus {
    this.rows[source] = { plays: 0, notInLibrary: 0, shortPlays: 0, updatedAt: null, removed: true, error: null };
    return this.status();
  }

  /** A source turned back on can come in again. */
  follow() {
    const s = this.settings();
    if (s.includePlexHistory) this.rows.plexHistory.removed = false;
    if (s.includeTautulliHistory) this.rows.tautulli.removed = false;
    if (s.includeLastfmHistory) this.rows.lastfm.removed = false;
  }
}
