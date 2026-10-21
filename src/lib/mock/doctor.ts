/**
 * Mock of the library doctor (docs/IPC.md "Library doctor"): a few findings
 * of each kind made from the fixture library, the checks and cover size
 * from Settings, ignores, and findings that go away once their fix runs.
 */
import type { Album, DoctorFinding, DoctorKind, DoctorReport, Settings, TagEditFile, TagValues, Track } from "../types";
import { ARCHIVE_ARTIST_ID, mockPlexMissed, type Fixtures } from "./fixtures";

const CHECK_OF: Record<DoctorKind, keyof Settings["doctorChecks"]> = {
  duplicate: "duplicates",
  cover: "covers",
  albumTags: "albumTags",
  splitAlbum: "splitAlbums",
  albumArtist: "albumArtist",
  lowQuality: "lowQuality",
  unmatched: "unmatched",
  soundsLike: "soundsLike",
};

const ORDER: DoctorKind[] = ["duplicate", "cover", "albumTags", "splitAlbum", "albumArtist", "lowQuality", "unmatched", "soundsLike"];

const q = (s: string) => `“${s}”`;

function length(ms: number): string {
  const s = Math.round(ms / 1000);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

export class MockDoctor {
  /** Ignored finding ids (the sync store in the app). */
  readonly ignored = new Map<string, { kind: DoctorKind; title: string }>();
  /** Findings whose fix ran. */
  readonly fixed = new Set<string>();
  /** Test hook: a check is going. */
  running = false;
  private built: { finding: DoctorFinding; coverPx?: number }[] | null = null;

  constructor(
    private fx: Fixtures,
    private settings: () => Settings,
  ) {}

  private tracksOf(a: Album): Track[] {
    return this.fx.tracksByAlbum.get(a.id) ?? [];
  }

  private finding(kind: DoctorKind, id: string, a: Album, title: string, text: string, fix: DoctorFinding["fix"]): DoctorFinding {
    return {
      id,
      kind,
      artistId: a.artistId,
      artist: a.artistTitle,
      albumId: a.id,
      album: a.title,
      thumb: a.thumb,
      title,
      text,
      evidence: [],
      fix,
      tracks: [],
      others: [],
      table: [],
    };
  }

  /** The findings, made once from the fixtures. */
  private build() {
    if (this.built) return this.built;
    const out: { finding: DoctorFinding; coverPx?: number }[] = [];
    const usable = this.fx.albums.filter((a) => a.artistId !== ARCHIVE_ARTIST_ID && this.tracksOf(a).length >= 2);
    const used = new Set<string>();
    const take = (pred: (a: Album) => boolean = () => true): Album => {
      const a = usable.find((x) => !used.has(x.id) && pred(x)) ?? usable[0];
      used.add(a.id);
      return a;
    };
    /** Two albums by one artist. */
    const pair = (): [Album, Album] => {
      const byArtist = new Map<string, Album[]>();
      for (const a of usable) if (!used.has(a.id)) byArtist.set(a.artistId, [...(byArtist.get(a.artistId) ?? []), a]);
      const list = [...byArtist.values()].find((l) => l.length >= 2) ?? [usable[0], usable[1]];
      used.add(list[0].id);
      used.add(list[1].id);
      return [list[0], list[1]];
    };

    // Unmatched first: the album Plex missed must not be taken for something else.
    const missed = this.fx.albums.find((a) => mockPlexMissed(this.fx.albums, a));
    for (const a of [missed ?? take(), take()]) {
      used.add(a.id);
      const f = this.finding(
        "unmatched",
        `unmatched:${a.id}`,
        a,
        `Plex didn't match ${a.title}`,
        "MusicBrainz has this release, but your server couldn't match it, so it's missing details like its cover choices and release date. Fix match lets you pick the right release.",
        "match",
      );
      f.evidence = ["Plex couldn't match it to a release.", "MusicBrainz has a release by this artist with this title.", "No release type or ARCHIVE_VERSION in the files."];
      out.push({ finding: f });
    }

    for (const a of [take(), take()]) {
      const t = this.tracksOf(a);
      const f = this.finding(
        "duplicate",
        `duplicate:${a.id}`,
        a,
        `${q(t[0].title)} is on this album twice`,
        "Your library has more than one file of the same recording here, so it plays twice. Open the album to compare the copies, then remove the one you don't want from your music folder.",
        "openAlbum",
      );
      f.evidence = [`${q(t[0].title)} is here twice with the same ISRC: ${length(t[0].durationMs)}, FLAC 16-bit/44.1 kHz; ${length(t[0].durationMs)}, MP3 320 kbps.`];
      f.tracks = [t[0].id, t[1].id];
      out.push({ finding: f });
    }

    const covers: [Album, number | null][] = [
      [take(), null],
      [take(), 300],
      [take(), 500],
      [take(), 800],
    ];
    for (const [a, px] of covers) {
      const f = this.finding(
        "cover",
        `cover:${a.id}`,
        a,
        px === null ? "No cover" : `Small cover (${px} × ${px})`,
        px === null
          ? "This album has no cover. Change cover lets you pick one from Plex's choices or use your own image."
          : "The cover is smaller than the size in Settings, so it looks soft on big screens and in full window. Change cover finds a bigger one.",
        "cover",
      );
      if (px === null) f.thumb = null;
      f.evidence = [px === null ? "This album has no cover." : `The cover is ${px} by ${px} pixels.`];
      out.push({ finding: f, coverPx: px ?? 0 });
    }

    for (const [i, a] of [take(), take()].entries()) {
      const t = this.tracksOf(a);
      const f = this.finding(
        "albumTags",
        `albumTags:${a.id}`,
        a,
        i === 0 ? "The songs don't agree on the album name and year" : "The songs don't agree on the album artist",
        i === 0
          ? "The files on this album have different tags, which can make Plex show the album wrong or split it. Fix the tags gives every song the same album name and year, and shows each change before it's saved."
          : "The files on this album have different album artists, which can make Plex show the album wrong or split it. Fix the tags gives every song the same album artist, and shows each change before it's saved.",
        "tags",
      );
      const odd = t.length - 1;
      f.table = t.map((s, n) => ({
        trackId: s.id,
        title: s.title,
        album: i === 0 && n === odd ? `${a.title} (Deluxe)` : a.title,
        albumArtist: i === 1 && n === odd ? `${a.artistTitle} & Friends` : a.artistTitle,
        year: i === 0 && n === odd ? (a.year ?? 2016) + 1 : a.year,
      }));
      f.set = i === 0 ? { album: a.title, albumArtist: null, year: a.year ?? 2016 } : { album: null, albumArtist: a.artistTitle, year: null };
      f.evidence =
        i === 0
          ? [
              `Album name: ${q(a.title)} on ${t.length - 1} songs, ${q(`${a.title} (Deluxe)`)} on 1 song.`,
              `Year: ${q(String(a.year ?? 2016))} on ${t.length - 1} songs, ${q(String((a.year ?? 2016) + 1))} on 1 song.`,
              `Read from ${t.length} of the album's ${t.length} files.`,
            ]
          : [`Album artist: ${q(a.artistTitle)} on ${t.length - 1} songs, ${q(`${a.artistTitle} & Friends`)} on 1 song.`, `Read from ${t.length} of the album's ${t.length} files.`];
      out.push({ finding: f });
    }

    for (let i = 0; i < 2; i++) {
      const [a, b] = pair();
      const f = this.finding(
        "splitAlbum",
        `splitAlbum:${[a.id, b.id].sort().join("+")}`,
        a,
        `${a.title} is split in two`,
        "Plex shows one album as 2, with the songs divided between them. Merging puts them back together as one album, and you can split it again later.",
        "mergeAlbums",
      );
      const na = this.tracksOf(a).length;
      f.evidence = [`2 albums are called ${q(a.title)} by ${a.artistTitle}.`, `One has songs 1 to ${na}.`, `One has songs ${na + 1} to ${na + this.tracksOf(b).length}.`, "They share no songs, and none is tagged as another edition or version."];
      const first = (x: Album) => this.tracksOf(x)[0]?.title ?? "";
      f.albumNote = `${na} songs, from ${q(first(a))}`;
      f.others = [{ id: b.id, title: a.title, thumb: b.thumb, note: `${this.tracksOf(b).length} songs, from ${q(first(b))}` }];
      out.push({ finding: f });
    }

    for (const a of [take(), take()]) {
      const t = this.tracksOf(a);
      const f = this.finding(
        "albumArtist",
        `albumArtist:${a.id}`,
        a,
        `No album artist on ${t.length - 1} of ${t.length} songs`,
        "Without an album artist, Plex files each song under its own artist and can break the album up. Fix the tags sets one album artist on every song, and shows each change before it's saved.",
        "tags",
      );
      f.table = t.map((s, n) => ({ trackId: s.id, title: s.title, album: a.title, albumArtist: n === 0 ? a.artistTitle : null, year: a.year }));
      f.evidence = [`${t.length - 1} of the ${t.length} files read ${t.length - 1 === 1 ? "has" : "have"} no album artist tag.`, `The songs credit ${a.artistTitle} and Guest Voice.`];
      f.set = { album: null, albumArtist: a.artistTitle, year: null };
      out.push({ finding: f });
    }

    for (let i = 0; i < 2; i++) {
      const [worse, better] = pair();
      const t = this.tracksOf(worse).slice(0, i === 0 ? 1 : 3);
      const f = this.finding(
        "lowQuality",
        `lowQuality:${worse.id}>${better.id}`,
        worse,
        t.length === 1 ? `${q(t[0].title)} has a better copy on ${better.title}` : `${t.length} songs have a better copy on ${better.title}`,
        `Your library has ${t.length === 1 ? "this song" : "these songs"} in better quality on ${better.title}. Play that copy instead, or look for a better copy of this album.`,
        "openAlbum",
      );
      f.evidence = t.map((s) => `${q(s.title)}: MP3 128 kbps here, FLAC 16-bit/44.1 kHz on ${better.title}.`);
      f.tracks = t.map((s) => s.id);
      f.others = [{ id: better.id, title: better.title, thumb: better.thumb }];
      out.push({ finding: f });
    }

    {
      // Shown only while identify by sound is on (it needs its results).
      const a = take();
      const t = this.tracksOf(a)[0];
      const f = this.finding(
        "soundsLike",
        `soundsLike:${t.id}:0f0e0d0c-0b0a-4909-8807-060504030201`,
        a,
        "Untagged or mislabelled file, sounds like Night Drive by Ana",
        `${t.title} sounds like Night Drive by Ana. See the tags that recording suggests next to the file's own; nothing is changed, so a tagger is where to fix them.`,
        "suggestedTags",
      );
      f.tracks = [t.id];
      f.evidence = [
        "Its audio fingerprint matched the MusicBrainz recording Night Drive by Ana (94% match, id 0f0e0d0c-0b0a-4909-8807-060504030201).",
        `The file is tagged as ${t.title} by ${a.artistTitle}.`,
        "Only the fingerprint and the file's length were sent to AcoustID to find this.",
      ];
      f.suggest = {
        trackId: t.id,
        title: t.title,
        artist: a.artistTitle,
        album: a.title,
        newTitle: "Night Drive",
        newArtist: "Ana",
        newAlbum: "Lights",
        recordingMbid: "0f0e0d0c-0b0a-4909-8807-060504030201",
        score: 94,
      };
      out.push({ finding: f });
    }

    this.built = out;
    return out;
  }

  /** Every finding for the checks that are on, in page order. */
  all(): DoctorFinding[] {
    const s = this.settings();
    if (s.libraryDoctor === false) return [];
    const checks = s.doctorChecks ?? { duplicates: true, covers: true, albumTags: true, splitAlbums: true, albumArtist: true, lowQuality: true, unmatched: true, soundsLike: true };
    const px = s.doctorCoverPx ?? 600;
    return this.build()
      .filter((b) => checks[CHECK_OF[b.finding.kind]] !== false)
      .filter((b) => b.finding.kind !== "soundsLike" || s.identifyBySound === true)
      .filter((b) => b.coverPx === undefined || b.coverPx < px)
      .filter((b) => !this.fixed.has(b.finding.id))
      .map((b) => b.finding)
      .sort((a, b) => ORDER.indexOf(a.kind) - ORDER.indexOf(b.kind) || a.artist.localeCompare(b.artist) || a.album.localeCompare(b.album));
  }

  report(): DoctorReport {
    const all = this.all();
    const findings = all.filter((f) => !this.ignored.has(f.id));
    const ignored = all.filter((f) => this.ignored.has(f.id));
    const total = this.fx.tracks.length;
    return {
      on: this.settings().libraryDoctor !== false,
      findings,
      ignored,
      count: findings.length,
      running: this.running,
      checkedAt: Math.floor(Date.now() / 1000) - 5 * 60,
      tagsRead: Math.round(total * 0.9),
      tagsTotal: total,
    };
  }

  count(): number {
    return this.report().count;
  }

  ignore(id: string, kind: DoctorKind, title: string, ignore: boolean) {
    if (!id.trim()) throw { kind: "invalid", message: "No finding to ignore." };
    if (ignore) this.ignored.set(id, { kind, title });
    else this.ignored.delete(id);
  }

  /** A fix ran on this album: its findings of that fix go away. Returns whether any did. */
  resolve(albumId: string, fix: DoctorFinding["fix"]): boolean {
    let any = false;
    for (const { finding } of this.build()) {
      const hit = finding.albumId === albumId || finding.others.some((o) => o.id === albumId);
      if (hit && finding.fix === fix && !this.fixed.has(finding.id)) {
        this.fixed.add(finding.id);
        any = true;
      }
    }
    return any;
  }

  /** The tags a song's file starts with, so tag findings show in the editor too. */
  tagsOf(trackId: string): TagValues | undefined {
    for (const { finding } of this.build()) {
      const r = finding.table.find((x) => x.trackId === trackId);
      if (r) return { album: r.album ?? "", albumArtist: r.albumArtist ?? "", year: r.year === null ? "" : String(r.year) };
    }
    return undefined;
  }

  /** Tags were saved on an album: its tag findings whose values every file now has go away. */
  resolveTags(albumId: string, files: TagEditFile[]): boolean {
    let any = false;
    for (const { finding } of this.build()) {
      if (finding.albumId !== albumId || finding.fix !== "tags" || !finding.set || this.fixed.has(finding.id)) continue;
      const s = finding.set;
      const want: [keyof TagValues, string | null][] = [
        ["album", s.album],
        ["albumArtist", s.albumArtist],
        ["year", s.year === null ? null : String(s.year)],
      ];
      if (files.every((f) => want.every(([k, v]) => v === null || (f.values[k] ?? "") === v))) {
        this.fixed.add(finding.id);
        any = true;
      }
    }
    return any;
  }

  /** Back to the starting findings (tests). */
  reset() {
    this.ignored.clear();
    this.fixed.clear();
    this.running = false;
  }
}
