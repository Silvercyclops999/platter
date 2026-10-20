/**
 * Mock "Bring music over": what bringing a file over does, in miniature. A
 * picked or dropped file is told apart by its name: a zip, JSON, the
 * Music app's XML library or a CSV/M3U list come in; anything else is
 * refused in the core's words. `scenario` picks what the file holds.
 */
import type { BringAlbum, BringStatus, ImportSourceId, ImportedRow } from "../importTypes";
import type { AppError } from "../types";

export type BringScenario = "some" | "none" | "all" | "empty";

const UNKNOWN_FILE =
  "Platter doesn't know that file. It reads a streaming service's data download (the zip), the Music app's library file, and CSV or M3U lists.";

const invalid = (message: string): AppError => ({ kind: "invalid", message });

const ALBUMS: [string, string, number, number][] = [
  ["Low Tide", "Shoreline", 9, 7],
  ["Echo Field", "Static", 6, 5],
  ["The Paper Kites", "Twelvefour", 5, 4],
  ["Hollow Coves", "Moments", 4, 3],
  ["Novo Amor", "Birthplace", 3, 2],
  ["Bon Iver", "22, A Million", 2, 1],
  ["Phoebe Bridgers", "Punisher", 2, 0],
];

function sourceOf(name: string): ImportSourceId | null {
  const ext = name.toLowerCase().split(".").pop() ?? "";
  if (ext === "xml") return "apple";
  if (ext === "zip" || ext === "json") return "streaming";
  if (["csv", "tsv", "txt", "m3u", "m3u8"].includes(ext)) return "lists";
  return null;
}

export class MockBring {
  scenario: BringScenario = "some";
  private st: BringStatus = MockBring.empty();
  /** Folded "artist|album" keys Lidarr can't find. */
  notFound = new Set<string>(["phoebe bridgers\u001fpunisher"]);

  static empty(): BringStatus {
    return {
      summary: { sources: [], liked: { total: 0, found: 0 }, songs: { total: 0, found: 0 }, playlists: 0, missingSongs: 0, missingAlbums: 0, unsure: 0, sentence: "" },
      lists: [],
      albums: [],
      progress: { toGet: 0, onTheWay: 0, here: 0, notFound: 0 },
      lidarr: { ready: false, practiceUntil: null },
      sent: { liked: 0, playlistsMade: 0, songsAdded: 0 },
    };
  }

  /** Test hook: nothing brought over, the default file. */
  reset() {
    this.scenario = "some";
    this.st = MockBring.empty();
    this.notFound = new Set(["phoebe bridgers\u001fpunisher"]);
  }

  status(lidarrReady: boolean): BringStatus {
    this.st.lidarr = { ready: lidarrReady, practiceUntil: null };
    this.st.progress = progress(this.st.albums);
    return structuredClone(this.st);
  }

  read(name: string, lidarrReady: boolean): BringStatus {
    const source = sourceOf(name);
    if (!source) throw invalid(UNKNOWN_FILE);
    if (this.scenario === "empty") throw invalid("Those files had no songs, playlists or plays in them.");
    this.bring(source);
    return this.status(lidarrReady);
  }

  adopt(source: ImportSourceId, lidarrReady: boolean): BringStatus {
    this.bring(source);
    return this.status(lidarrReady);
  }

  private bring(source: ImportSourceId) {
    const albums: BringAlbum[] =
      this.scenario === "all"
        ? []
        : ALBUMS.map(([artist, album, songs, liked]) => ({
            key: `${artist.toLowerCase()}\u001f${album.toLowerCase()}`,
            artist,
            album,
            songs,
            liked: this.scenario === "none" ? songs : liked,
            state: "missing",
            reason: null,
            releaseGroupId: null,
          }));
    const missingSongs = albums.reduce((n, a) => n + a.songs, 0);
    const likedTotal = 900;
    const likedFound = this.scenario === "all" ? 900 : this.scenario === "none" ? 0 : 812;
    const sentence =
      this.scenario === "all"
        ? "All 900 liked songs are already in your library."
        : this.scenario === "none"
          ? `None of the 900 liked songs are in your library yet. ${missingSongs} songs from ${albums.length} albums aren't.`
          : `812 of 900 liked songs are already in your library. ${missingSongs} songs from ${albums.length} albums aren't.`;
    const sources = this.st.summary.sources.includes(source) ? this.st.summary.sources : [...this.st.summary.sources, source];
    this.st = {
      ...this.st,
      summary: {
        sources,
        liked: { total: likedTotal, found: likedFound },
        songs: { total: 1040, found: this.scenario === "all" ? 1040 : this.scenario === "none" ? 0 : 1009 },
        playlists: 3,
        missingSongs: this.scenario === "all" ? 0 : missingSongs,
        missingAlbums: albums.length,
        unsure: this.scenario === "some" ? 4 : 0,
        sentence,
      },
      lists: [
        { name: "Road Trip", importId: "pl:1", playlistId: this.scenario === "none" ? null : "mock-bring-1", gone: false, total: 48, found: this.scenario === "none" ? 0 : 41 },
        { name: "Rainy Sundays", importId: "pl:2", playlistId: this.scenario === "none" ? null : "mock-bring-2", gone: false, total: 62, found: this.scenario === "none" ? 0 : 60 },
        { name: "Gym", importId: "pl:3", playlistId: this.scenario === "none" ? null : "mock-bring-3", gone: false, total: 30, found: this.scenario === "none" ? 0 : 30 },
      ],
      albums,
      sent: this.scenario === "none" ? { liked: 0, playlistsMade: 0, songsAdded: 0 } : { liked: likedFound, playlistsMade: 3, songsAdded: 131 },
    };
  }

  get(keys: string[] | null, lidarrReady: boolean): BringStatus {
    if (!lidarrReady) throw invalid("Set up Lidarr first.");
    for (const a of this.st.albums) {
      const pick = keys ? keys.includes(a.key) && a.state !== "here" : a.state === "missing";
      if (!pick) continue;
      if (this.notFound.has(a.key)) {
        a.state = "notFound";
        a.reason = "Lidarr couldn't find it. MusicBrainz has no official release of it.";
      } else {
        a.state = "asked";
        a.reason = null;
      }
    }
    return this.status(lidarrReady);
  }

  /** An asked album arrives (tests and the demo). */
  arrive(key: string) {
    const a = this.st.albums.find((x) => x.key === key);
    if (a) a.state = "here";
  }

  waitingRows(playlistId: string): ImportedRow[] {
    const l = this.st.lists.find((x) => x.playlistId === playlistId);
    if (!l) return [];
    return this.st.albums
      .filter((a) => a.state !== "here")
      .slice(0, Math.max(0, l.total - l.found))
      .map((a, i) => ({ songId: 9000 + i, artist: a.artist, title: `${a.album} (track ${i + 1})`, album: a.album, durationMs: 200_000, status: "missing", track: null }));
  }
}

function progress(albums: BringAlbum[]) {
  const p = { toGet: 0, onTheWay: 0, here: 0, notFound: 0 };
  for (const a of albums) {
    if (a.state === "missing") p.toGet++;
    else if (a.state === "picking" || a.state === "asked") p.onTheWay++;
    else if (a.state === "here") {
      p.onTheWay++;
      p.here++;
    } else p.notFound++;
  }
  return p;
}
