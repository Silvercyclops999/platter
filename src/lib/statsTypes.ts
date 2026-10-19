/** Listening stats (mirrors src-tauri/src/stats). Times are ms; days are local "YYYY-MM-DD". */
import type { Track } from "./types";

/** Days to count, both ends included. No ends: all time. */
export interface StatsRange {
  from: string | null;
  to: string | null;
}

export interface StatsTotals {
  ms: number;
  plays: number;
  songs: number;
  artists: number;
  albums: number;
  /** Days with at least one play. */
  days: number;
}

export interface StatsPoint {
  /** "YYYY-MM-DD" for days, "YYYY-MM" for months. */
  key: string;
  plays: number;
  ms: number;
}

export interface StatsStreak {
  days: number;
  from: string;
  to: string;
}

export interface TopSong {
  key: string;
  trackId: string | null;
  title: string;
  artist: string;
  album: string | null;
  plays: number;
  ms: number;
  /** Counts in your own stats; never leaves the device in anything shared. */
  unreleased: boolean;
  track: Track | null;
}

export interface TopArtist {
  key: string;
  artistId: string | null;
  name: string;
  thumb: string | null;
  plays: number;
  ms: number;
}

/** "Same artist as...": an artist counted as another in stats, Wrapped and top lists. */
export interface ArtistAlias {
  /** The artist's stats key (its name in lower case). */
  key: string;
  name: string;
  /** The other artist's key. */
  sameAs: string;
  sameAsName: string;
  /** Unix seconds. */
  addedAt: number;
}

/** A played artist, for the "Same artist as..." picker. */
export interface PlayedArtist {
  key: string;
  name: string;
  plays: number;
}

export interface TopAlbum {
  key: string;
  albumId: string | null;
  title: string;
  artist: string;
  thumb: string | null;
  plays: number;
  ms: number;
  unreleased: boolean;
}

/** One earlier year's plays on today's date (`home_on_this_day`). */
export interface OnThisDayYear {
  year: number;
  /** "YYYY-MM-DD". */
  date: string;
  plays: number;
  ms: number;
  /** Most played first; songs no longer in the library have no trackId. */
  songs: TopSong[];
  albums: TopAlbum[];
  /** What the card plays: library songs only. */
  trackIds: string[];
  covers: string[];
}

export interface OnThisDay {
  /** False while the setting is off. */
  shown: boolean;
  /** Today, "YYYY-MM-DD". */
  date: string;
  /** The most played day first. */
  years: OnThisDayYear[];
}

/** A Time travel card: a decade's mix or your year's (`home_time_travel`). */
export interface HistoryMix {
  /** "decade:1990" or "year:2021". */
  id: string;
  kind: "decade" | "year";
  /** The decade's first year, or the year. */
  year: number;
  plays: number;
  trackIds: string[];
  covers: string[];
}

export interface TimeTravel {
  shown: boolean;
  yourYear: HistoryMix | null;
  /** Oldest first. */
  decades: HistoryMix[];
}

export interface TopGenre {
  name: string;
  plays: number;
  ms: number;
  /** The cover of the range's most played album in the genre. */
  thumb: string | null;
}

export interface StatsOverview {
  from: string;
  to: string;
  totals: StatsTotals;
  bucket: "day" | "month";
  series: StatsPoint[];
  streak: StatsStreak | null;
  /** Listening time per hour of the day. */
  hours: number[];
  /** Listening time per weekday, 0 is Sunday. */
  weekdays: number[];
  busiestHour: number | null;
  busiestWeekday: number | null;
  topSongs: TopSong[];
  topArtists: TopArtist[];
  topAlbums: TopAlbum[];
  topGenres: TopGenre[];
}

/** What the range pickers offer. */
export interface StatsSpan {
  first: string | null;
  last: string | null;
  /** Months with plays, newest first ("YYYY-MM"). */
  months: string[];
  today: string;
}

export type StatsTopKind = "songs" | "artists" | "albums" | "genres";
export type StatsItemKind = "song" | "artist" | "album";

export interface PlayedFrom {
  /** album | playlist | artist | mix | radio | search | tracks | liked | imported */
  kind: string;
  id: string | null;
  title: string | null;
}

export interface FirstListen {
  at: number;
  song: string;
  trackId: string | null;
  from: PlayedFrom | null;
}

export interface ItemStats {
  kind: StatsItemKind;
  plays: number;
  ms: number;
  first: FirstListen | null;
  lastAt: number | null;
  /** Place in your all-time top for its kind. */
  rank: number | null;
  /** Plays per month, the last 12 months, oldest first. */
  series: StatsPoint[];
}

export interface MonthRecap {
  /** "YYYY-MM". */
  month: string;
  totals: StatsTotals;
  prevMs: number;
  topSongs: TopSong[];
  topArtists: TopArtist[];
  topAlbum: TopAlbum | null;
  /** The month's most played albums (up to 4), for its covers. */
  topAlbums: TopAlbum[];
  topGenre: string | null;
  daily: StatsPoint[];
  biggestDay: StatsPoint | null;
}

/** A song at a moment of a range; its plays and minutes are that day's. */
export interface MomentSong {
  song: TopSong;
  /** "YYYY-MM-DD". */
  day: string;
  /** Unix ms of the play, when it's one play. */
  at: number | null;
}

/** Small stories from a range. */
export interface StatsMoments {
  /** The range's first song. */
  firstSong: MomentSong | null;
  /** The day one song was played most (3 or more times). */
  replayed: MomentSong | null;
  /** The artist first played in the range that you played most. */
  discovery: { artist: TopArtist; day: string } | null;
  /** A song played again after a break of 30 days or more (the longest). */
  comeback: { song: MomentSong; gapDays: number } | null;
  /** Artists first played in the range; null for all time. */
  newArtists: number | null;
}

export interface WeightedArtist {
  name: string;
  weight: number;
}

export interface WeightedSong {
  artist: string;
  title: string;
  weight: number;
}

/** What one side shares to compare: top names with weights, no history. */
export interface CompareSummary {
  version: number;
  name: string | null;
  artists: WeightedArtist[];
  songs: WeightedSong[];
  unreleasedOk: boolean;
}

export interface CommonArtist {
  name: string;
  artistId: string | null;
  thumb: string | null;
  mine: number;
  theirs: number;
}

export interface CompareResult {
  friend: string | null;
  /** 0 to 100. */
  overlap: number;
  commonArtists: CommonArtist[];
  theirSongsNew: { artist: string; title: string; track: Track | null }[];
}

/** Which Top Songs lists and Wrapped years there are (`top_songs_index`). */
export interface TopSongsIndex {
  /** Years with a list, newest first. */
  years: number[];
  /** Any plays at all: the All-Time list. */
  allTime: boolean;
  /** The year Home shows (early December to mid January), or null. */
  promoted: number | null;
  /** Years with plays for Wrapped on the profile, newest first. */
  wrappedYears: number[];
  currentYear: number;
}

export interface TopSongsRow {
  track: Track;
  plays: number;
  ms: number;
  unreleased: boolean;
  /** The year's first play (yearly lists only). */
  firstOfYear: boolean;
}

export interface TopSongsList {
  /** null: All-Time. */
  year: number | null;
  title: string;
  /** Most played first; the position is the rank. */
  rows: TopSongsRow[];
  durationMs: number;
}

export type TimeOfDay = "morning" | "afternoon" | "evening" | "night";

/** Shares are 0 to 1. */
export interface Personality {
  time: TimeOfDay;
  timeShare: number;
  /** Plays on songs played 5 times or more that year. */
  repeatShare: number;
  repeats: boolean;
  /** Plays on songs first heard that year. */
  newShare: number;
  mostlyNew: boolean;
}

export interface Wrapped {
  year: number;
  totals: StatsTotals;
  /** Listening time the year before. */
  prevMs: number;
  topArtists: TopArtist[];
  topSongs: TopSong[];
  topAlbums: TopAlbum[];
  topGenres: TopGenre[];
  personality: Personality | null;
  hours: number[];
  biggestDay: StatsPoint | null;
  /** The song played most in one day; its plays are that day's. */
  replayed: MomentSong | null;
  firstSong: MomentSong | null;
  newArtists: number;
}
