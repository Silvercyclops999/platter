/**
 * Data rules for the now-playing panel, kept apart from the components so
 * they can be tested on their own: the details fetch (with a small cache),
 * the credits list, the next-in-queue pick, the context link and the colours
 * taken from the cover.
 */
import { useEffect, useState } from "react";
import { artUrl, backend } from "../lib/backend";
import { headerColorsFromImageUrl, type HeaderColors } from "../lib/color";
import { creditKey, type ArtistLookup, type Credit } from "../lib/credits";
import type { AlbumCreditRow, Contributor, CreditRole, NowPlayingDetails, QueueContextInfo, QueueItem, QueueView } from "../lib/types";
import type { Route } from "../stores/router";

// ---------------------------------------------------------------- details

/** Details for the last few tracks, so skipping back and forth doesn't refetch. */
const CACHE_SIZE = 30;
const cache = new Map<string, NowPlayingDetails>();

export function clearNowPlayingCache() {
  cache.clear();
}

/** Details for a track; null while loading or when they couldn't be read. */
export function useNowPlayingDetails(trackId: string | null | undefined): { details: NowPlayingDetails | null; loading: boolean } {
  const [state, setState] = useState<{ id: string | null; details: NowPlayingDetails | null; loading: boolean }>(() => ({
    id: trackId ?? null,
    details: trackId ? (cache.get(trackId) ?? null) : null,
    loading: !!trackId && !cache.has(trackId),
  }));
  // A new track: show what the cache has at once (no stale details from the last one).
  if ((trackId ?? null) !== state.id) {
    const hit = trackId ? cache.get(trackId) : undefined;
    setState({ id: trackId ?? null, details: hit ?? null, loading: !!trackId && !hit });
  }
  useEffect(() => {
    if (!trackId || cache.has(trackId)) return;
    let alive = true;
    backend.nowPlayingDetails(trackId).then(
      (d) => {
        cache.set(trackId, d);
        if (cache.size > CACHE_SIZE) cache.delete(cache.keys().next().value!);
        if (alive) setState({ id: trackId, details: d, loading: false });
      },
      () => alive && setState({ id: trackId, details: null, loading: false }),
    );
    return () => {
      alive = false;
    };
  }, [trackId]);
  return { details: state.details, loading: state.loading };
}

// ---------------------------------------------------------------- credits

export interface CreditRow {
  name: string;
  /** Library artist id, when the name is a library artist. */
  id: string | null;
  roles: string[];
}

const ROLE_LABEL: Record<CreditRole, string> = {
  composer: "Composer",
  writer: "Writer",
  lyricist: "Lyricist",
  producer: "Producer",
};

/**
 * The credits card's rows: the track's artists first (the first as main
 * artist), then anyone else Plex credits, with roles merged by name.
 */
export function buildCredits(trackCredits: Credit[], extra: Contributor[], lookup: ArtistLookup): CreditRow[] {
  const rows: CreditRow[] = trackCredits.map((c, i) => ({ name: c.name, id: c.id, roles: [i === 0 ? "Main artist" : "Artist"] }));
  for (const c of extra) {
    const key = creditKey(c.name);
    if (!key) continue;
    let row = rows.find((r) => creditKey(r.name) === key);
    if (!row) {
      row = { name: c.name.trim(), id: lookup.get(key) ?? null, roles: [] };
      rows.push(row);
    }
    for (const role of c.roles) {
      const label = ROLE_LABEL[role] ?? role;
      if (!row.roles.includes(label)) row.roles.push(label);
    }
  }
  return rows;
}

/** Labels for MusicBrainz credit roles. */
export const MB_ROLE_LABEL: Record<string, string> = {
  producer: "Producer",
  writer: "Writer",
  composer: "Composer",
  lyricist: "Lyricist",
  featured: "Featured",
  mixer: "Mixing",
  engineer: "Engineer",
  mastering: "Mastering",
};

/**
 * Add a song's MusicBrainz credits to the card's rows, merged by name. A
 * featured artist already in the artist line keeps just that row.
 */
export function withAlbumCredits(rows: CreditRow[], extra: AlbumCreditRow[] | undefined): CreditRow[] {
  if (!extra?.length) return rows;
  const out = rows.map((r) => ({ ...r, roles: [...r.roles] }));
  for (const c of extra) {
    const key = creditKey(c.name);
    if (!key) continue;
    let row = out.find((r) => creditKey(r.name) === key);
    const roles = c.roles.filter((r) => !(row && r === "featured")).map((r) => MB_ROLE_LABEL[r] ?? r);
    if (!roles.length) continue;
    if (!row) {
      row = { name: c.name.trim(), id: c.artistId, roles: [] };
      out.push(row);
    }
    for (const label of roles) if (!row.roles.includes(label)) row.roles.push(label);
  }
  return out;
}

/** The card is only worth showing when it says more than the artist line does. */
export function creditsWorthShowing(rows: CreditRow[]): boolean {
  return rows.length > 1 || (rows.length === 1 && rows[0].roles.length > 1);
}

/** Rows shown before "Show all". */
export const CREDITS_PREVIEW = 3;

// ---------------------------------------------------------------- queue and context

/** What plays next: the user's queue first, then the context. */
export function nextInQueue(queue: QueueView | null | undefined): QueueItem | null {
  return queue?.userQueue[0] ?? queue?.nextUp[0] ?? null;
}

/** Where the context name in the header links to, if anywhere. */
export function contextRoute(ctx: QueueContextInfo | null | undefined): Route | null {
  if (!ctx?.id) return null;
  switch (ctx.kind) {
    case "album":
      return { name: "album", id: ctx.id };
    case "playlist":
      return { name: "playlist", id: ctx.id };
    case "artist":
      return { name: "artist", id: ctx.id };
    case "liked":
      return { name: "liked" };
    default:
      return null;
  }
}

// ---------------------------------------------------------------- lyrics

/** Lines in the lyrics card before it fades out. */
export const LYRICS_PREVIEW_LINES = 5;

/** The first lines with words in them, for the lyrics card. */
export function lyricsPreview(lines: string[], n = LYRICS_PREVIEW_LINES): string[] {
  return lines.filter((l) => l.trim() !== "").slice(0, n);
}

// ---------------------------------------------------------------- colour

/** Colours from the cover (the same pick as page headers); null until read or when it can't be. */
export function useCoverColors(thumb: string | null | undefined): HeaderColors | null {
  const [colors, setColors] = useState<{ thumb: string | null; c: HeaderColors | null }>({ thumb: null, c: null });
  useEffect(() => {
    let alive = true;
    const url = artUrl(thumb, 64);
    if (!url) return;
    headerColorsFromImageUrl(url).then((c) => alive && setColors({ thumb: thumb ?? null, c }));
    return () => {
      alive = false;
    };
  }, [thumb]);
  // Keep the last colour until the new cover's arrives, so the change can ease.
  return thumb ? colors.c : null;
}
