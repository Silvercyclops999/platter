/**
 * The listening diary for the mock: two dozen entries over the last
 * fourteen months, some rated and noted, heard on headphones or speakers.
 * Albums at an even place in the fixtures were first heard long ago.
 */
import type { Fixtures } from "./fixtures";
import type { DiaryEdit, DiaryItem, DiaryList, DiaryQuery } from "../types";

const DAY = 86_400_000;

const NOTES = [
  "Side two at night with the lights off. The bass on the closer is enormous.",
  "Better than I remembered. The middle stretch drags a little.",
  "",
  "First proper listen on the new headphones. Every breath on the vocal is there.",
  "",
  "Rainy Sunday record.",
];
const OUTPUTS = ["Headphones (HD 600)", "Speakers (KEF LSX)", null];

export class MockDiary {
  private entries = new Map<string, DiaryItem>();
  private seq = 0;

  constructor(private fx: Fixtures) {
    this.reset();
  }

  private albums() {
    return this.fx.albums.filter((a) => a.section !== "demos" && (this.fx.tracksByAlbum.get(a.id)?.length ?? 0) >= 3);
  }

  reset() {
    this.entries.clear();
    const albums = this.albums();
    if (!albums.length) return;
    const now = Date.now();
    for (let i = 0; i < 24; i++) {
      const a = albums[(i * 7) % albums.length];
      const n = this.fx.tracksByAlbum.get(a.id)?.length ?? 0;
      const output = OUTPUTS[i % OUTPUTS.length];
      const at = now - i * 18 * DAY - (3 + (i % 5)) * 3_600_000;
      const auto = i % 6 !== 5;
      this.put({
        id: auto ? `auto:${a.id}:${1000 + i}` : `log:${a.id}:${at}:0`,
        albumId: a.id,
        title: a.title,
        artist: a.artistTitle,
        year: a.year,
        at,
        startedAt: auto ? at - n * 4 * 60_000 : null,
        songsPlayed: auto ? n - (i % 4 === 1 ? 1 : 0) : 0,
        songsTotal: n,
        minutes: auto ? n * 4 : 0,
        device: auto ? "Den PC" : null,
        output: auto ? output : null,
        rating: i % 3 === 2 ? null : [4.5, 3.5, 5, 4, 2.5][i % 5],
        note: NOTES[i % NOTES.length],
        auto,
        thumb: a.thumb,
        inLibrary: true,
        listenedOn: auto && output ? (output.startsWith("Head") ? "headphones" : "speakers") : null,
      });
    }
  }

  /** Empty the diary (a fresh install). */
  clear() {
    this.entries.clear();
  }

  private put(e: DiaryItem) {
    this.entries.set(e.id, e);
  }

  list(q: DiaryQuery | null | undefined): DiaryList {
    const all = [...this.entries.values()];
    const words = (q?.text ?? "").toLowerCase().split(/\s+/).filter(Boolean);
    const entries = all
      .filter((e) => {
        if (q?.from != null && e.at < q.from) return false;
        if (q?.until != null && e.at >= q.until) return false;
        if (q?.rated && e.rating == null) return false;
        if (q?.withNotes && !e.note.trim()) return false;
        if (q?.albumId && e.albumId !== q.albumId) return false;
        const hay = `${e.note}\n${e.title}\n${e.artist}`.toLowerCase();
        return words.every((w) => hay.includes(w));
      })
      .sort((a, b) => b.at - a.at || (a.id < b.id ? 1 : -1));
    const times = all.map((e) => e.at);
    return {
      entries,
      total: all.length,
      firstAt: times.length ? Math.min(...times) : null,
      lastAt: times.length ? Math.max(...times) : null,
    };
  }

  edit(id: string, edit: DiaryEdit): DiaryItem {
    const e = this.entries.get(id);
    if (!e) throw { kind: "notFound", message: "That diary entry is gone." };
    const r = edit.rating;
    if (r != null && (r < 0.5 || r > 5 || (r * 2) % 1 !== 0)) throw { kind: "invalid", message: "A rating is from half a star to five stars." };
    const note = edit.note.trim();
    if ([...note].length > 500) throw { kind: "invalid", message: "A note can be up to 500 characters." };
    if (edit.at != null && edit.at > Date.now() + DAY) throw { kind: "invalid", message: "That date is in the future." };
    const next = { ...e, rating: r, note, at: edit.at ?? e.at };
    this.put(next);
    return next;
  }

  delete(id: string): boolean {
    return this.entries.delete(id);
  }

  log(albumId: string): DiaryItem {
    const a = this.fx.albums.find((x) => x.id === albumId);
    if (!a) throw { kind: "notFound", message: "Album not found" };
    const at = Date.now();
    const e: DiaryItem = {
      id: `log:${a.id}:${at}:${this.seq++}`,
      albumId: a.id,
      title: a.title,
      artist: a.artistTitle,
      year: a.year,
      at,
      startedAt: null,
      songsPlayed: 0,
      songsTotal: this.fx.tracksByAlbum.get(a.id)?.length ?? 0,
      minutes: 0,
      device: null,
      output: null,
      rating: null,
      note: "",
      auto: false,
      thumb: a.thumb,
      inLibrary: true,
      listenedOn: null,
    };
    this.put(e);
    return e;
  }

  firstHeard(albumId: string): number | null {
    const i = this.fx.albums.findIndex((a) => a.id === albumId);
    const played = i >= 0 && i % 2 === 0 ? Date.now() - (i + 2) * 97 * DAY : null;
    const logged = [...this.entries.values()].filter((e) => e.albumId === albumId).map((e) => e.at);
    const all = [...(played == null ? [] : [played]), ...logged];
    return all.length ? Math.min(...all) : null;
  }

  /** What an export would write (the mock has no save dialog). */
  exportText(q: DiaryQuery | null | undefined): string {
    return this.list(q)
      .entries.map((e) => `${new Date(e.at).toDateString()}  ${e.title}, ${e.artist}${e.rating ? `  ${e.rating}/5` : ""}${e.note ? `\n  ${e.note}` : ""}`)
      .join("\n");
  }
}
