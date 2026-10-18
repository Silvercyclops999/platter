/**
 * Home led by records for the mock backend (`home_records`). The core works
 * these out from the listen log and the stats day tables (db/records.rs);
 * the mock picks stable albums from the fixtures so every shelf and the
 * hero have something to show.
 */
import type { Album, RecordHome, RecordProgress, Settings } from "../types";
import type { Fixtures } from "./fixtures";

const SHELF_N = 12;
const DAY = 86_400_000;

export const EMPTY_RECORD_HOME: RecordHome = { shown: false, resume: null, newArrivals: [], spunThisWeek: [], unfinished: [], notInAYear: [] };

function progress(fx: Fixtures, album: Album, into: number, playedAt: number): RecordProgress | null {
  const tracks = fx.tracksByAlbum.get(album.id) ?? [];
  if (tracks.length < 4) return null;
  const next = Math.min(tracks.length - 1, Math.max(1, Math.round(tracks.length * into)));
  return { album, nextTrackId: tracks[next].id, nextTitle: tracks[next].title, nextNumber: next + 1, total: tracks.length, playedAt };
}

export function mockRecordHome(fx: Fixtures, settings: Settings, now = Date.now()): RecordHome {
  if (settings.homeRecordsFirst === false) return EMPTY_RECORD_HOME;
  const albums = fx.albums.filter((a) => a.thumb);
  const byPlays = [...albums].sort((a, b) => b.viewCount - a.viewCount || a.id.localeCompare(b.id));
  const partway = byPlays
    .slice(2, 40)
    .map((a, i) => progress(fx, a, 0.3 + ((i * 7) % 5) / 10, now - (i + 1) * 0.6 * DAY))
    .filter((p): p is RecordProgress => !!p);
  const [resume = null, ...unfinished] = partway;
  return {
    shown: true,
    resume,
    newArrivals: [...albums].sort((a, b) => b.addedAt - a.addedAt || a.id.localeCompare(b.id)).slice(0, SHELF_N),
    spunThisWeek: byPlays.slice(0, 9).map((album, i) => ({ album, plays: Math.max(2, 14 - i * 2) })),
    unfinished: unfinished.slice(0, 7),
    notInAYear: byPlays
      .slice(-30)
      .reverse()
      .slice(0, 8)
      .map((album, i) => ({ album, plays: Math.max(5, 30 - i * 3), lastPlayed: now - (400 + i * 45) * DAY })),
  };
}
