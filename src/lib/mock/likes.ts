/**
 * Likes for the mock backend: the same rules as the core
 * (src-tauri/src/likes.rs) on the fixture tracks. A like is a Plex rating of
 * 10, unliking clears it. Writes land at once, as if the server were always
 * reachable; the test hooks fake Plex refusing a write and a like made in
 * another Plex app.
 */
import type { LikedMeans, LikedPage, Track } from "../types";
import type { Fixtures } from "./fixtures";

export const LIKED_RATING = 10;
const HOUR = 3600;
const DAY = 86_400;

/** Lowest rating that counts as liked. */
export const likedMin = (means: LikedMeans) => (means === "fourUp" ? 8 : LIKED_RATING);

export class MockLikes {
  private refuseNext = false;

  constructor(
    private fx: Fixtures,
    private means: () => LikedMeans,
    /** These tracks changed (emits likes://changed). */
    private changed: (ids: string[]) => void,
    private toastError: (message: string) => void,
  ) {
    // Track genres come from their album; liked songs get a spread of
    // "date added" values (hours, days, weeks, months ago); a few songs
    // carry three or four stars from another Plex app.
    const now = Math.floor(Date.now() / 1000);
    const albumGenres = new Map(fx.albums.map((a) => [a.id, a.genres]));
    const steps = [3 * HOUR, 23 * HOUR, 2 * DAY, 5 * DAY, 9 * DAY, 20 * DAY, 45 * DAY, 120 * DAY, 300 * DAY];
    let n = 0;
    fx.tracks.forEach((t, i) => {
      const extra = i % 5 === 0 ? ["Late night"] : i % 7 === 0 ? ["Upbeat"] : [];
      t.genres = [...(albumGenres.get(t.albumId) ?? []), ...extra];
      if (t.userRating === LIKED_RATING) {
        t.ratedAt = now - (steps[n % steps.length] + n * 1800);
        n++;
      } else if (i % 37 === 3) {
        t.userRating = i % 2 ? 6 : 8;
        t.ratedAt = now - 60 * DAY;
      } else {
        t.userRating = null;
        t.ratedAt = null;
      }
    });
  }

  private liked(t: Track): boolean {
    return (t.userRating ?? 0) >= likedMin(this.means());
  }

  private byId(id: string): Track {
    const t = this.fx.tracks.find((x) => x.id === id);
    if (!t) throw { kind: "notFound", message: "Track not found" };
    return t;
  }

  set(trackIds: string[], liked: boolean): string[] {
    if (!trackIds.length) throw { kind: "invalid", message: "No songs to change." };
    const tracks = trackIds.map((id) => this.byId(id));
    const refuse = this.refuseNext;
    this.refuseNext = false;
    const changed: Track[] = [];
    const before = new Map<string, Pick<Track, "userRating" | "ratedAt">>();
    const now = Math.floor(Date.now() / 1000);
    for (const t of tracks) {
      if (this.liked(t) === liked || before.has(t.id)) continue;
      before.set(t.id, { userRating: t.userRating, ratedAt: t.ratedAt ?? null });
      t.userRating = liked ? LIKED_RATING : null;
      t.ratedAt = liked ? now : null;
      changed.push(t);
    }
    const ids = changed.map((t) => t.id);
    if (ids.length) this.changed(ids);
    if (refuse && changed.length) {
      // "Plex" refuses: back as it was, with a toast, a moment later.
      setTimeout(() => {
        for (const t of changed) Object.assign(t, before.get(t.id));
        const name = `"${changed[0].title}"`;
        this.toastError(
          liked ? `Your server didn't save the like for ${name}. It's back as it was.` : `Your server didn't remove ${name} from Liked Songs. It's back as it was.`,
        );
        this.changed(ids);
      }, 0);
    }
    return ids;
  }

  likedTracks(): Track[] {
    return this.fx.tracks.filter((t) => this.liked(t)).sort((a, b) => (b.ratedAt ?? 0) - (a.ratedAt ?? 0));
  }

  ids(): string[] {
    return this.likedTracks().map((t) => t.id);
  }

  page(): LikedPage {
    const tracks = this.likedTracks();
    return { tracks, totalDurationMs: tracks.reduce((s, t) => s + t.durationMs, 0) };
  }

  playlistsWith(trackId: string): string[] {
    return this.fx.playlists
      .filter((p) => (this.fx.playlistTracks.get(p.id) ?? []).includes(trackId))
      .sort((a, b) => (b.updatedAt ?? 0) - (a.updatedAt ?? 0) || a.title.localeCompare(b.title))
      .map((p) => p.id);
  }

  // ---------------------------------------------------------------- test hooks

  /** The next like or unlike reaches "Plex", which refuses it. */
  refuseNextWrite() {
    this.refuseNext = true;
  }

  /** Another Plex app liked (or unliked) this song; it arrives with sync. */
  likeElsewhere(trackId: string, liked = true) {
    const t = this.byId(trackId);
    t.userRating = liked ? LIKED_RATING : null;
    t.ratedAt = liked ? Math.floor(Date.now() / 1000) : null;
    this.changed([trackId]);
  }

  /** Clear every like (the empty Liked Songs page). */
  clearAll() {
    const ids = this.ids();
    for (const t of this.fx.tracks) {
      if (ids.includes(t.id)) {
        t.userRating = null;
        t.ratedAt = null;
      }
    }
    if (ids.length) this.changed(ids);
  }
}
