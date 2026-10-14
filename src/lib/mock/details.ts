/**
 * Invented now-playing details for the mock backend: artist bios, extra
 * credits and lyrics. Deterministic per track, and deliberately patchy like a
 * real library: some artists have no bio, most tracks have no extra credits,
 * and only some have lyrics.
 */
import type { Artist, Contributor, Lyrics, NowPlayingDetails, Track } from "../types";
import { hashString, mulberry32 } from "./fixtures";

const BIO_OPENINGS = [
  "started out recording on a borrowed four-track in a converted boathouse",
  "grew out of late-night sessions in a shared rehearsal space above a bakery",
  "began as a bedroom project and slowly turned into a full touring band",
  "came together after a chance meeting at a small coastal folk festival",
];
const BIO_MIDDLES = [
  "Their records lean on warm tape hiss, patient arrangements and lyrics about leaving and coming home.",
  "Early releases were passed around on hand-numbered cassettes before a small label picked them up.",
  "Over the years the sound moved from sparse acoustic sketches to wide, layered studio pieces.",
  "Each album was written in a different town, and the places tend to show up in the songs.",
];
const BIO_ENDINGS = [
  "They still self-produce most of their work and release a handful of demos between albums.",
  "Live, the songs are often stretched out into long improvised codas.",
  "A collected edition of their unreleased material has been rumoured for years.",
];

const PEOPLE = ["M. Hale", "June Park", "Theo Vasquez", "Lena Okafor", "R. Castellan", "Wim de Bruin"];

const LYRIC_LINES = [
  "Lights along the harbour wall",
  "Counting every passing car",
  "I kept the window open wide",
  "So the weather could come inside",
  "Paper boats on a silver tide",
  "We said we'd leave when the summer's done",
  "Now the summer's gone and we're still here",
  "Holding on to the radio",
  "Every station plays the same old song",
  "Turn it up and sing along",
  "There's a light that never sleeps",
  "Down where the river runs too deep",
  "Call me when the morning breaks",
  "I'll be standing by the gate",
];

/** No bio for every fourth artist (by id), like artists Plex has no data for. */
export function mockArtistSummary(artist: Artist): string | null {
  const h = hashString(artist.id);
  if (h % 4 === 3) return null;
  const r = mulberry32(h);
  const pick = <T,>(xs: T[]) => xs[Math.floor(r() * xs.length)];
  return `${artist.title} ${pick(BIO_OPENINGS)}. ${pick(BIO_MIDDLES)} ${pick(BIO_ENDINGS)}`;
}

function mockLyrics(r: () => number): Lyrics {
  const verse = () => Array.from({ length: 4 }, () => LYRIC_LINES[Math.floor(r() * LYRIC_LINES.length)]);
  const lines = [...verse(), "", ...verse(), "", ...verse()];
  const synced = r() < 0.5;
  return { synced, lines, credit: synced ? "Written by Demo Writer. Lyrics licensed and provided by LyricFind" : undefined };
}

function mockCredits(track: Track, r: () => number): Contributor[] {
  // About a third of tracks credit a composer or writer.
  if (r() > 0.35) return [];
  const credits: Contributor[] = [{ name: track.artistTitle, roles: ["composer"] }];
  const other = PEOPLE[Math.floor(r() * PEOPLE.length)];
  credits.push({ name: other, roles: r() < 0.5 ? ["writer"] : ["composer", "lyricist"] });
  if (r() < 0.4) credits.push({ name: PEOPLE[(PEOPLE.indexOf(other) + 1) % PEOPLE.length], roles: ["producer"] });
  return credits;
}

export function mockNowPlayingDetails(track: Track, artist: Artist | undefined): NowPlayingDetails {
  const r = mulberry32(hashString(`np-${track.id}`));
  const hasLyrics = r() < 0.6;
  return {
    trackId: track.id,
    artist: artist
      ? { id: artist.id, title: artist.title, summary: mockArtistSummary(artist), thumb: artist.thumb, art: artist.art }
      : null,
    credits: mockCredits(track, r),
    lyrics: hasLyrics ? mockLyrics(r) : null,
  };
}
