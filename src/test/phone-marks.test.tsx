/**
 * The phone's row marks: one small download glyph (filled when the song or
 * item is on the phone, a ring with progress while it comes) and the
 * explicit "E", lined up before the grey line on song rows, album rows,
 * search hits and now playing, and the same glyph in album headers.
 */
import { act, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { backend } from "../lib/backend";
import type { DownloadItem, DownloadStatus, Track } from "../lib/types";
import { RowGlyphs } from "../phone/bits";
import { DownloadMark, downloadMarkLabel } from "../phone/DownloadMark";
import { PhoneDownload } from "../phone/PhoneDownload";
import { SongRow } from "../phone/pages/common";
import { downloadItemOf } from "../phone/pages/Library";
import { resetDownloads, useDownloads } from "../stores/downloads";
import { useFileTags } from "../stores/fileTags";

const EMPTY_TAGS = { tracks: {}, albums: {}, groups: {} };

function status(items: DownloadItem[], current?: DownloadStatus["current"]): DownloadStatus {
  return { items, usedBytes: 0, limitBytes: 0, folder: "", queued: 0, current: current ?? null };
}
const item = (patch: Partial<DownloadItem> = {}): DownloadItem => ({ kind: "album", id: "a1", title: "Glass Season", state: "downloading", total: 10, done: 4, failed: 0, missing: 0, progress: 0.4, bytes: 0, ...patch });

let track: Track;
beforeEach(async () => {
  resetDownloads();
  useFileTags.setState({ index: EMPTY_TAGS });
  track = (await backend.albumPage("1000")).tracks[0];
});
afterEach(() => {
  resetDownloads();
  useFileTags.setState({ index: EMPTY_TAGS });
});

describe("the download mark", () => {
  it("says Downloaded, or how far it has come", () => {
    expect(downloadMarkLabel("done", 1)).toBe("Downloaded");
    expect(downloadMarkLabel("downloading", 0.404)).toBe("Downloading, 40%");
    expect(downloadMarkLabel("waiting", 0)).toBe("Downloading, 0%");
    expect(downloadMarkLabel("none", 0)).toBeNull();
    expect(downloadMarkLabel("error", 0)).toBeNull();
  });

  it("is a filled glyph when done and a ring while downloading, never words", () => {
    const { container, rerender } = render(<DownloadMark state="done" />);
    expect(screen.getByRole("img", { name: "Downloaded" }).querySelector(".dl-glyph--filled")).not.toBeNull();
    rerender(<DownloadMark state="downloading" progress={0.25} />);
    const ring = screen.getByRole("img", { name: "Downloading, 25%" });
    expect(ring.querySelector(".dl-ring")).not.toBeNull();
    expect(container.textContent).toBe("");
  });

  it("a song row shows it before the artist line, with its own progress while it comes", () => {
    const { rerender } = render(<ul><SongRow track={track} onPlay={() => {}} /></ul>);
    expect(screen.queryByTestId("download-mark")).toBeNull();
    act(() => useDownloads.setState({ status: status([], { trackId: track.id, received: 50, total: 100 }) }));
    rerender(<ul><SongRow track={track} onPlay={() => {}} /></ul>);
    expect(screen.getByRole("img", { name: "Downloading, 50%" })).toBeInTheDocument();
    act(() => useDownloads.setState({ ids: new Set([track.id]), status: status([]) }));
    const mark = screen.getByRole("img", { name: "Downloaded" });
    expect(mark.closest(".ph-row__sub")).not.toBeNull();
    expect(mark.closest(".ph-glyphs")).not.toBeNull();
  });

  it("a library row follows its album's download, and Liked Songs its own", () => {
    expect(downloadItemOf({ kind: "album", id: "a1" })).toEqual({ kind: "album", id: "a1" });
    expect(downloadItemOf({ kind: "playlist", id: "p1", liked: true })).toEqual({ kind: "liked", id: "" });
    expect(downloadItemOf({ kind: "artist", id: "r1" })).toBeUndefined();
    act(() => useDownloads.setState({ status: status([item()]) }));
    const { rerender } = render(<RowGlyphs item={{ kind: "album", id: "a1" }} />);
    expect(screen.getByRole("img", { name: "Downloading, 40%" })).toBeInTheDocument();
    act(() => useDownloads.setState({ status: status([item({ done: 10, progress: 1, state: "downloaded" })]) }));
    rerender(<RowGlyphs item={{ kind: "album", id: "a1" }} />);
    expect(screen.getByRole("img", { name: "Downloaded" })).toBeInTheDocument();
  });

  it("the album header uses the same glyph, with no count beside it", () => {
    act(() => useDownloads.setState({ status: status([item()]) }));
    render(<PhoneDownload kind="album" id="a1" title="Glass Season" />);
    const button = screen.getByRole("button", { name: "Downloading, 40%. Stop downloading Glass Season" });
    expect(button.querySelector(".dl-ring")).not.toBeNull();
    expect(button.textContent).toBe("");
    act(() => useDownloads.setState({ status: status([item({ done: 10, progress: 1, state: "downloaded" })]) }));
    const done = screen.getByRole("button", { name: "Remove download of Glass Season" });
    expect(done.querySelector(".dl-glyph--filled")).not.toBeNull();
  });
});

describe("the explicit mark", () => {
  it("shows on an explicit song's row, before the download mark", () => {
    act(() => {
      useFileTags.setState({ index: { ...EMPTY_TAGS, tracks: { [track.id]: { explicit: true } } } });
      useDownloads.setState({ ids: new Set([track.id]) });
    });
    render(<ul><SongRow track={track} onPlay={() => {}} /></ul>);
    const glyphs = within(screen.getByRole("listitem")).getAllByRole("img");
    expect(glyphs.map((g) => g.getAttribute("aria-label"))).toEqual(["Explicit", "Downloaded"]);
  });

  it("shows on an explicit album's row and nowhere else", () => {
    act(() => useFileTags.setState({ index: { ...EMPTY_TAGS, albums: { a1: { explicit: true } } } }));
    const { rerender } = render(<RowGlyphs albumId="a1" />);
    expect(screen.getByRole("img", { name: "Explicit" })).toBeInTheDocument();
    rerender(<RowGlyphs albumId="a2" />);
    expect(screen.queryByRole("img", { name: "Explicit" })).toBeNull();
  });
});
