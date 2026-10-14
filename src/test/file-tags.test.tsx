import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { archiveQualityLabel, fileQualityClass, versionLabel } from "../components/FileTagBadges";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { skippedMessage } from "../lib/mock/fileTags";
import { DEFAULT_SETTINGS, type Edition, type TagScanStatus, type ToastEvent } from "../lib/types";
import { TrackList, TrackRow } from "../components/TrackRow";
import { AlbumPage } from "../pages/AlbumPage";
import { AlbumCard } from "../pages/cards";
import { editionName } from "../pages/EditionPicker";
import { scanNote } from "../pages/FileTagSettings";
import { SettingsPage } from "../pages/SettingsPage";
import { connectFileTags, editionsOf, useFileTags } from "../stores/fileTags";
import { useRouter } from "../stores/router";
import { connectPlayer, usePlayer } from "../stores/player";
import { PlayerBar } from "../shell/PlayerBar";
import { loadSettings, updateSettings } from "../stores/settings";
import { body } from "./rows";

const mock = backend as MockBackend;
let disconnect: (() => void) | null = null;

beforeEach(async () => {
  await backend.settingsSet({ ...DEFAULT_SETTINGS });
  await loadSettings();
  disconnect = connectFileTags(mock);
  await waitFor(() => expect(Object.keys(useFileTags.getState().index.tracks).length).toBeGreaterThan(0));
});
afterEach(() => {
  disconnect?.();
  disconnect = null;
});

const group = () => Object.values(mock.tags.index().groups)[0];
const status = (p: Partial<TagScanStatus>): TagScanStatus => ({
  enabled: true,
  running: false,
  done: 0,
  total: 1439,
  failed: 0,
  paused: false,
  ...p,
});

describe("file tag helpers", () => {
  it("names versions, and released shows nothing", () => {
    expect(versionLabel("leak")).toBe("Leak");
    expect(versionLabel(" Demo ")).toBe("Demo");
    expect(versionLabel("unreleased")).toBe("Unreleased");
    expect(versionLabel("released")).toBeNull();
    expect(versionLabel("something else")).toBeNull();
    expect(versionLabel(undefined)).toBeNull();
  });

  it("shows source quality only when it differs from the file", () => {
    const flac = { codec: "flac", bitrate: 1000 };
    const mp3 = { codec: "mp3", bitrate: 320 };
    const low = { codec: "mp3", bitrate: 128 };
    expect(fileQualityClass(flac)).toBe("lossless");
    expect(fileQualityClass(mp3)).toBe("cdq");
    expect(fileQualityClass(low)).toBe("lq");
    expect(archiveQualityLabel("lossless", flac)).toBeNull();
    expect(archiveQualityLabel("lq", flac)).toBe("LQ");
    expect(archiveQualityLabel("lossless", mp3)).toBe("Lossless");
    expect(archiveQualityLabel("cdq", mp3)).toBeNull();
    expect(archiveQualityLabel("snippet", mp3)).toBeNull();
  });

  it("says how far the scan is, and when it's paused or done", () => {
    expect(scanNote(status({ done: 812 }))).toBe("Reading tags: 812 of 1,439.");
    expect(scanNote(status({ done: 1439, failed: 3 }))).toBe("Tags read from all 1,439 songs. 3 couldn't be read.");
    expect(scanNote(status({ done: 20, paused: true }))).toMatch(/^Paused at 20 of 1,439/);
    expect(scanNote(status({ enabled: false }))).toBeUndefined();
    expect(scanNote(null)).toBeUndefined();
  });

  it("names editions by tag, then title, with the year when two clash", () => {
    const e = (albumId: string, label: string | null, title = "Same", year: number | null = 2020): Edition => ({ albumId, title, label, year });
    const all = [e("1", "Standard"), e("2", null, "Same", 2021), e("3", null, "Same", 2022)];
    expect(editionName(all[0], all)).toBe("Standard");
    expect(editionName(all[1], all)).toBe("Same (2021)");
  });

  it("uses the core's words for skipped songs", () => {
    expect(skippedMessage(1, 0)).toBe("Skipped 1 explicit song. Explicit content is filtered in Settings.");
    expect(skippedMessage(0, 2)).toBe("Skipped 2 fan or AI edits.");
    expect(skippedMessage(2, 1)).toBe("Skipped 2 explicit songs and 1 fan or AI edit.");
    expect(skippedMessage(0, 0)).toBeNull();
  });
});

describe("badges", () => {
  it("rows show E before the artist line, or after the title on album pages", async () => {
    const album = mock.fx.albums[0];
    render(<AlbumPage id={album.id} />);
    const first = mock.fx.tracksByAlbum.get(album.id)![0];
    const row = await screen.findByRole("row", { name: new RegExp(`^${first.title}, explicit by`) });
    expect(within(row).getByRole("img", { name: "Explicit" })).toHaveTextContent("E");
    const second = mock.fx.tracksByAlbum.get(album.id)![1];
    const plain = body(screen.getAllByRole("row")).find((r) => r.dataset.trackId === second.id)!;
    expect(within(plain).queryByRole("img", { name: "Explicit" })).toBeNull();
  });

  it("unreleased builds show version badges", async () => {
    const build = mock.fx.albums.find((a) => a.format === null && !mock.tags.hiddenAlbums().has(a.id))!;
    render(<AlbumPage id={build.id} />);
    const tracks = mock.fx.tracksByAlbum.get(build.id)!;
    await screen.findAllByRole("row").then(body);
    const rowOf = (id: string) => body(screen.getAllByRole("row")).find((r) => r.dataset.trackId === id)!;
    expect(within(rowOf(tracks[0].id)).getByText("Leak")).toHaveAttribute("title", "Version: Leak");
    expect(within(rowOf(tracks[tracks.length - 1].id)).getByText("Snippet")).toBeInTheDocument();
  });

  it("rows show the source quality when it differs from the file", () => {
    const build = mock.fx.albums.find((a) => a.format === null)!;
    const tracks = mock.fx.tracksByAlbum.get(build.id)!;
    render(
      <TrackList label="Songs">
        {tracks.map((t, i) => (
          <TrackRow key={t.id} track={t} index={i + 1} onPlay={() => {}} />
        ))}
      </TrackList>,
    );
    const rowOf = (id: string) => body(screen.getAllByRole("row")).find((r) => r.dataset.trackId === id)!;
    // The first song is a lossy file tagged as from a lossless source; the snippet is tagged LQ.
    expect(fileQualityClass(tracks[0])).not.toBe("lossless");
    expect(within(rowOf(tracks[0].id)).getByText("Lossless")).toHaveAttribute("title", "Source quality: Lossless");
    expect(within(rowOf(tracks[tracks.length - 1].id)).getByText("LQ")).toBeInTheDocument();
    expect(within(rowOf(tracks[1].id)).queryByText(/Lossless|LQ|CDQ/)).toBeNull();
  });

  it("the player bar shows E for an explicit song", async () => {
    const album = mock.fx.albums[0];
    const first = mock.fx.tracksByAlbum.get(album.id)![0];
    await backend.playContext({ context: { kind: "album", id: album.id }, startTrackId: first.id });
    await backend.playPause();
    const off = connectPlayer(mock);
    try {
      render(<PlayerBar />);
      await waitFor(() => expect(usePlayer.getState().state?.track?.id).toBe(first.id));
      const title = screen.getByRole("link", { name: first.title }).parentElement!;
      expect(within(title).getByRole("img", { name: "Explicit" })).toBeInTheDocument();
    } finally {
      off();
    }
  });

  it("released songs show no version badge", async () => {
    const album = mock.fx.albums[1];
    render(<AlbumPage id={album.id} />);
    await screen.findAllByRole("row").then(body);
    for (const v of ["Leak", "Demo", "Snippet", "Session"]) expect(screen.queryByText(v)).toBeNull();
  });
});

describe("editions", () => {
  it("the group shows as one card that counts its editions", async () => {
    const [face, other] = group();
    const page = await backend.albums({ sort: "name", offset: 0, limit: 500 });
    const ids = page.items.map((a) => a.id);
    expect(ids).toContain(face.albumId);
    expect(ids).not.toContain(other.albumId);
    const album = mock.fx.albums.find((a) => a.id === face.albumId)!;
    render(<AlbumCard album={album} />);
    expect(screen.getByText(/· 2 editions$/)).toBeInTheDocument();
    expect(editionsOf(useFileTags.getState().index, face.albumId)).toHaveLength(2);
  });

  it("grouping off lists every edition again", async () => {
    await updateSettings({ groupEditions: false });
    const [, other] = group();
    const page = await backend.albums({ sort: "name", offset: 0, limit: 500 });
    expect(page.items.map((a) => a.id)).toContain(other.albumId);
    const album = mock.fx.albums.find((a) => a.id === other.albumId)!;
    render(<AlbumCard album={album} />);
    expect(screen.queryByText(/editions/)).toBeNull();
  });

  it("the album page picker switches to the other edition", async () => {
    const user = userEvent.setup();
    const [face, other] = group();
    act(() => useRouter.getState().reset({ name: "album", id: face.albumId }));
    render(<AlbumPage id={face.albumId} />);
    const picker = await screen.findByRole("combobox", { name: "Edition" });
    expect(within(picker).getAllByRole("option").map((o) => o.textContent)).toEqual(["Build v1", "Build v2"]);
    expect(picker).toHaveValue(face.albumId);
    await user.selectOptions(picker, other.albumId);
    const r = useRouter.getState();
    expect(r.entries[r.index].route).toEqual({ name: "album", id: other.albumId });
    expect(r.entries).toHaveLength(1);
  });

  it("albums outside a group have no picker", async () => {
    render(<AlbumPage id={mock.fx.albums[0].id} />);
    await screen.findAllByRole("row").then(body);
    expect(screen.queryByRole("combobox", { name: "Edition" })).toBeNull();
  });
});

describe("explicit filter and fan edits", () => {
  let toasts: ToastEvent[] = [];
  let un: (() => void) | null = null;
  beforeEach(async () => {
    toasts = [];
    un = await backend.listen("toast", (t) => toasts.push(t));
  });
  afterEach(() => un?.());

  it("filtered leaves explicit songs out of an album with one toast", async () => {
    await updateSettings({ explicitContent: "filtered" });
    const album = mock.fx.albums[0];
    await backend.playContext({ context: { kind: "album", id: album.id } });
    const s = await backend.playerState();
    const explicitId = mock.fx.tracksByAlbum.get(album.id)![0].id;
    expect(s.track?.id).not.toBe(explicitId);
    expect(s.queue.nextUp.map((i) => i.track.id)).not.toContain(explicitId);
    expect(toasts.filter((t) => /explicit/.test(t.message))).toHaveLength(1);
  });

  it("allowed plays everything without a toast", async () => {
    const album = mock.fx.albums[0];
    const first = mock.fx.tracksByAlbum.get(album.id)![0].id;
    await backend.playContext({ context: { kind: "album", id: album.id }, startTrackId: first });
    expect((await backend.playerState()).track?.id).toBe(first);
    expect(toasts).toHaveLength(0);
  });

  it("fan edits stay off album pages unless shown", async () => {
    const editId = Object.entries(mock.tags.index().tracks).find(([, b]) => b.edit)![0];
    const albumId = mock.fx.tracks.find((t) => t.id === editId)!.albumId;
    expect((await backend.albumPage(albumId)).tracks.map((t) => t.id)).not.toContain(editId);
    await updateSettings({ showFanEdits: true });
    expect((await backend.albumPage(albumId)).tracks.map((t) => t.id)).toContain(editId);
  });
});

describe("settings", () => {
  it("has the file tag rows under Library with their defaults and progress", async () => {
    render(<SettingsPage />);
    const lib = await screen.findByRole("region", { name: "Library" });
    expect(within(lib).getByRole("heading", { level: 3, name: "File tags" })).toBeInTheDocument();
    expect(within(lib).getByRole("switch", { name: "Read extra tags from files" })).toBeChecked();
    expect(within(lib).getByRole("combobox", { name: "Explicit content" })).toHaveValue("allowed");
    expect(within(lib).getByRole("switch", { name: "Show fan and AI edits" })).not.toBeChecked();
    expect(within(lib).getByRole("switch", { name: "Group album editions" })).toBeChecked();
    expect(await within(lib).findByText(/Tags read from all .* songs\. 2 couldn't be read\./)).toBeInTheDocument();
    act(() => useFileTags.getState().setScan(status({ done: 812, running: true })));
    expect(within(lib).getByRole("status")).toHaveTextContent("Reading tags: 812 of 1,439.");
  });

  it("each row saves its setting", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    await user.click(await screen.findByRole("switch", { name: "Read extra tags from files" }));
    await user.click(screen.getByRole("switch", { name: "Show fan and AI edits" }));
    await user.click(screen.getByRole("switch", { name: "Group album editions" }));
    await user.selectOptions(screen.getByRole("combobox", { name: "Explicit content" }), "filtered");
    const s = await backend.settingsGet();
    expect([s.readFileTags, s.showFanEdits, s.groupEditions, s.explicitContent]).toEqual([false, true, false, "filtered"]);
  });

  it("turning reading off says nothing about progress (app state: scan off)", async () => {
    await updateSettings({ readFileTags: false });
    act(() => useFileTags.getState().setScan(status({ enabled: false, done: 10 })));
    render(<SettingsPage />);
    const row = (await screen.findByRole("switch", { name: "Read extra tags from files" })).closest("[data-setting]")!;
    expect(within(row as HTMLElement).queryByRole("status")).toBeNull();
  });

  it("app state: a paused scan says so in Settings", async () => {
    act(() => useFileTags.getState().setScan(status({ done: 20, paused: true })));
    render(<SettingsPage />);
    expect(await screen.findByText(/Paused at 20 of 1,439: the server can't be reached directly/)).toBeInTheDocument();
  });
});
