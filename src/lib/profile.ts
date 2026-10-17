/**
 * The profile page's rules: which parts there are and who may see each,
 * this month as a range, the meta line, and the friends list built from the
 * people on your server and the friends who shared stats with you.
 */
import type { CompareSummary, StatsRange } from "./statsTypes";
import type { FriendActivity, Playlist } from "./types";

export type ProfileSection = "numbers" | "topArtists" | "topTracks" | "playlists" | "friends" | "links";

/** "you": only you ever see it. "shared": people you share your music with may see it. */
export type ProfileAudience = "you" | "shared";

/**
 * Your listening is yours alone; playlists and friends are for the people
 * you share with. Nothing on the profile is public, and nothing here sends
 * the profile anywhere: a view for friends would use `profileSections("friend")`.
 */
export const PROFILE_AUDIENCE: Record<ProfileSection, ProfileAudience> = {
  numbers: "you",
  topArtists: "you",
  topTracks: "you",
  playlists: "shared",
  friends: "shared",
  links: "you",
};

const ORDER: ProfileSection[] = ["numbers", "topArtists", "topTracks", "playlists", "friends", "links"];

/** The parts a viewer sees, in page order. */
export function profileSections(viewer: "me" | "friend"): ProfileSection[] {
  return viewer === "me" ? ORDER : ORDER.filter((s) => PROFILE_AUDIENCE[s] === "shared");
}

/** This month so far, from the 1st to `today` ("YYYY-MM-DD"). */
export function thisMonth(today: string): StatsRange {
  return { from: `${today.slice(0, 7)}-01`, to: today };
}

const count = (n: number, one: string, many = `${one}s`) => `${n.toLocaleString()} ${n === 1 ? one : many}`;

/** "12 playlists · 340 plays this month", leaving out what isn't known yet. */
export function metaLine(playlists: number | null, plays: number | null): string {
  const parts: string[] = [];
  if (playlists != null) parts.push(count(playlists, "playlist"));
  if (plays != null) parts.push(`${count(plays, "play")} this month`);
  return parts.join(" · ");
}

/** Playlists worth showing on a profile: your own, newest change first. */
export function profilePlaylists(all: Playlist[]): Playlist[] {
  return [...all].sort((a, b) => (b.updatedAt ?? 0) - (a.updatedAt ?? 0));
}

export interface ProfileFriend {
  name: string;
  thumb: string | null;
  /** They shared their stats summary: Compare works. */
  canCompare: boolean;
}

/** People on your server and friends who shared stats, once each (by name). */
export function profileFriends(activity: FriendActivity[], shared: CompareSummary[]): ProfileFriend[] {
  const out = new Map<string, ProfileFriend>();
  for (const f of activity) {
    const k = f.name.trim().toLowerCase();
    if (k && !out.has(k)) out.set(k, { name: f.name, thumb: f.thumb, canCompare: false });
  }
  for (const s of shared) {
    const k = s.name?.trim().toLowerCase();
    if (!k) continue;
    const had = out.get(k);
    out.set(k, had ? { ...had, canCompare: true } : { name: s.name!, thumb: null, canCompare: true });
  }
  return [...out.values()];
}
