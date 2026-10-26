/**
 * Downloads on the phone live in Library, the way playlists do: a
 * downloaded mix is a Library row with the playlists (with the mark and
 * "Mix · N songs") and under Downloaded, opening its page; album and
 * playlist search hits carry the mark; and removing anything that has a
 * song saved asks first, whatever its size, in the phone's words.
 */
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { DownloadDialogs } from "../components/DownloadButton";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import type { DownloadItem, DownloadStatus } from "../lib/types";
import { usePhoneForm } from "../phone/form";
import { phoneSubtitle } from "../phone/libraryRows";
import { PhoneDownload } from "../phone/PhoneDownload";
import { downloadItemOf, PhoneLibrary } from "../phone/pages/Library";
import { PhoneSearch } from "../phone/pages/Search";
import { mixEntry } from "../shell/useLibrary";
import { downloadRoute, resetDownloads, toggleDownload, useDownloads } from "../stores/downloads";
import { useLibraryView } from "../stores/library";
import { useRouter } from "../stores/router";
import { isMixSaved, resetSavedMixes, saveMix, useSavedMixes } from "../stores/savedMixes";
import { SaveMixButton } from "../phone/pages/Mix";
import { downloadingCount } from "../phone/pages/Library";
import { OnThisPhone } from "../phone/pages/Home";
import { useApp } from "../stores/app";
import { StorageNote } from "../pages/DownloadSettings";
import { setDownload } from "../stores/downloads";
import { vi } from "vitest";
import { storageParts } from "../pages/DownloadSettings";
import { OfflineCaption, HealthStrip } from "../phone/PhoneShell";
import { SongRow } from "../phone/pages/common";
import type { Health } from "../lib/types";

const mock = backend as MockBackend;
const SLOW = { timeout: 6000 };
const route = () => useRouter.getState().entries[useRouter.getState().index].route;

function status(items: DownloadItem[]): DownloadStatus {
  return { items, usedBytes: 0, limitBytes: 0, folder: "", queued: 0, current: null };
}
const mix = (patch: Partial<DownloadItem> = {}): DownloadItem => ({
  kind: "mix",
  id: "mix:moonlit-hours",
  title: "Moonlit Hours",
  state: "downloaded",
  total: 25,
  done: 25,
  failed: 0,
  missing: 0,
  progress: 1,
  bytes: 120 * 1024 ** 2,
  ...patch,
});

beforeEach(() => {
  usePhoneForm.setState({ phone: true });
  resetDownloads();
  resetSavedMixes();
  useLibraryView.getState().setFilter("all");
  useLibraryView.getState().setDownloaded(null);
  useRouter.getState().reset({ name: "home" });
});
afterEach(() => {
  usePhoneForm.setState({ phone: false });
  resetDownloads();
  resetSavedMixes();
  useLibraryView.getState().setDownloaded(null);
});

describe("a downloaded mix is in Library", () => {
  it("as a row like a playlist: the mix's own download, Mix and its song count", () => {
    const e = mixEntry(mix());
    expect(e).toMatchObject({ kind: "playlist", id: "mix:moonlit-hours", mix: true, downloaded: true, trackCount: 25 });
    expect(phoneSubtitle(e, false)).toBe("Mix · 25 songs");
    expect(phoneSubtitle(e, true)).toBe("25 songs");
    expect(downloadItemOf(e)).toEqual({ kind: "mix", id: "mix:moonlit-hours" });
    // Still coming: listed, but not downloaded yet.
    expect(mixEntry(mix({ done: 0, state: "queued" })).downloaded).toBeUndefined();
  });

  it("opens the mix's page, and Top Songs lists open Top Songs", () => {
    expect(downloadRoute({ kind: "mix", id: "mix:moonlit-hours" })).toEqual({ name: "mix", id: "mix:moonlit-hours" });
    expect(downloadRoute({ kind: "mix", id: "topSongs:all" })).toEqual({ name: "topSongs" });
    expect(downloadRoute({ kind: "mix", id: "topSongs:2025" })).toEqual({ name: "topSongs", year: 2025 });
    expect(downloadRoute({ kind: "liked", id: "" })).toEqual({ name: "liked" });
  });

  it("shows with the playlists and under Downloaded, and leaves when its download is removed", async () => {
    act(() => useDownloads.setState({ status: status([mix()]) }));
    render(<PhoneLibrary />);
    const list = await screen.findByRole("list", { name: "Your library" }, SLOW);
    const row = await within(list).findByRole("button", { name: /Moonlit Hours/ }, SLOW);
    expect(within(row).getByRole("img", { name: "Downloaded" })).toBeInTheDocument();
    expect(row).toHaveTextContent("Mix · 25 songs");

    const show = screen.getByRole("group", { name: "Show" });
    fireEvent.click(within(show).getByRole("button", { name: "Playlists" }));
    expect(await within(screen.getByRole("list", { name: "Your library" })).findByRole("button", { name: /Moonlit Hours/ })).toBeInTheDocument();
    fireEvent.click(within(show).getByRole("button", { name: "Playlists" }));

    act(() => useLibraryView.getState().setDownloaded(true));
    const only = screen.getByRole("list", { name: "Your library" });
    expect(within(only).getByRole("button", { name: /Moonlit Hours/ })).toBeInTheDocument();
    expect(within(only).queryByRole("button", { name: /^Liked Songs/ })).toBeNull();

    fireEvent.click(within(only).getByRole("button", { name: /Moonlit Hours/ }));
    await waitFor(() => expect(route()).toEqual({ name: "mix", id: "mix:moonlit-hours" }));

    act(() => useDownloads.setState({ status: status([]) }));
    await waitFor(() => expect(screen.queryByRole("button", { name: /Moonlit Hours/ })).toBeNull());
  });
});

describe("search hits", () => {
  it("album and playlist hits carry their download mark", async () => {
    const albums = (await backend.search({ query: "Slow Ghosts", filter: "all", limit: 20 })).albums;
    const playlists = (await backend.search({ query: "Late Night Drive", filter: "all", limit: 20 })).playlists;
    const a = albums.find((x) => x.title === "Slow Ghosts")!;
    const p = playlists.find((x) => x.title === "Late Night Drive")!;
    const done = { state: "downloaded" as const, total: 3, done: 3, failed: 0, missing: 0, progress: 1, bytes: 1 };
    act(() =>
      useDownloads.setState({
        status: status([
          { kind: "album", id: a.id, title: a.title, ...done },
          { kind: "playlist", id: p.id, title: p.title, ...done },
        ]),
      }),
    );
    for (const [query, title] of [
      ["Slow Ghosts", "Slow Ghosts"],
      ["Late Night Drive", "Late Night Drive"],
    ]) {
      useRouter.getState().reset({ name: "search", query });
      const { unmount } = render(<PhoneSearch />);
      const title_ = await screen.findByText(title, { selector: ".ph-row__title" }, SLOW);
      const row = title_.closest(".ph-row") as HTMLElement;
      expect(within(row).getByRole("img", { name: "Downloaded" })).toBeInTheDocument();
      unmount();
    }
  });
});

describe("removing on the phone", () => {
  it("asks first even for a small download; Cancel keeps it, Remove removes it", async () => {
    mock.downloads.seedDone("album", "1000", "Small Record");
    resetDownloads();
    const s = await backend.downloadsStatus();
    act(() => useDownloads.setState({ status: s }));
    const item = s.items.find((i) => i.kind === "album" && i.id === "1000")!;
    expect(item.bytes).toBeLessThan(1024 ** 3);
    render(
      <>
        <PhoneDownload kind="album" id="1000" title="Small Record" />
        <DownloadDialogs />
      </>,
    );
    fireEvent.click(screen.getByRole("button", { name: "Remove download of Small Record" }));
    const dialog = await screen.findByRole("dialog", { name: "Remove from downloads?" });
    expect(dialog).toHaveTextContent("You won't be able to play this offline.");
    expect(dialog).toHaveTextContent("on this phone.");
    fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(mock.downloads.isOn("album", "1000")).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "Remove download of Small Record" }));
    fireEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Remove" }));
    await waitFor(() => expect(mock.downloads.isOn("album", "1000")).toBe(false));
  });

  it("stopping one with nothing saved yet doesn't ask", () => {
    act(() => useDownloads.setState({ status: status([mix({ done: 0, state: "queued", bytes: 0, progress: 0 })]) }));
    toggleDownload({ kind: "mix", id: "mix:moonlit-hours", title: "Moonlit Hours" }, { askAlways: true });
    expect(useDownloads.getState().confirming).toBeNull();
    act(() => useDownloads.setState({ status: status([mix()]) }));
    toggleDownload({ kind: "mix", id: "mix:moonlit-hours", title: "Moonlit Hours" }, { askAlways: true });
    expect(useDownloads.getState().confirming?.target.id).toBe("mix:moonlit-hours");
  });
});

describe("a mix saved to Library", () => {
  it("downloading saves it, and removing the download leaves it saved", async () => {
    const t = { kind: "mix" as const, id: "mix:moonlit-hours", title: "Moonlit Hours", trackIds: () => ["1", "2", "3"] };
    toggleDownload(t, { askAlways: true });
    await waitFor(() => expect(isMixSaved("mix:moonlit-hours")).toBe(true));
    expect(useSavedMixes.getState().mixes[0]).toMatchObject({ id: "mix:moonlit-hours", title: "Moonlit Hours", trackCount: 3 });
    act(() => useDownloads.setState({ status: status([]) }));
    render(<PhoneLibrary />);
    const list = await screen.findByRole("list", { name: "Your library" }, SLOW);
    const row = await within(list).findByRole("button", { name: /Moonlit Hours/ }, SLOW);
    expect(row).toHaveTextContent("Mix · 3 songs");
    expect(within(row).queryByRole("img", { name: "Downloaded" })).toBeNull();
  });

  it("the add button saves it and takes it out; a downloaded one asks first, and its download goes too", async () => {
    render(
      <>
        <SaveMixButton id="mix:moonlit-hours" title="Moonlit Hours" trackCount={25} />
      </>,
    );
    fireEvent.click(screen.getByRole("button", { name: "Add Moonlit Hours to your library" }));
    expect(isMixSaved("mix:moonlit-hours")).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "Remove Moonlit Hours from your library" }));
    expect(isMixSaved("mix:moonlit-hours")).toBe(false);

    saveMix({ id: "mix:moonlit-hours", title: "Moonlit Hours", trackCount: 25 });
    act(() => useDownloads.setState({ status: status([mix()]) }));
    fireEvent.click(screen.getByRole("button", { name: "Remove Moonlit Hours from your library" }));
    const dialog = await screen.findByRole("dialog", { name: "Remove from your library?" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(isMixSaved("mix:moonlit-hours")).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "Remove Moonlit Hours from your library" }));
    fireEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Remove" }));
    expect(isMixSaved("mix:moonlit-hours")).toBe(false);
  });
});

describe("while songs come in", () => {
  it("Library counts them, only while one is actually coming", () => {
    const going = [mix({ state: "downloading", done: 3, total: 25 }), { ...mix({ id: "a1", state: "queued", done: 0, total: 15 }), kind: "album" as const }];
    expect(downloadingCount({ ...status(going), current: { trackId: "x", received: 1 } })).toEqual({ done: 3, total: 40 });
    expect(downloadingCount(status(going))).toBeNull();
    expect(downloadingCount({ ...status([mix()]), current: { trackId: "x", received: 1 } })).toBeNull();
  });

  it("shows the strip over the title", async () => {
    act(() => useDownloads.setState({ status: { ...status([mix({ state: "downloading", done: 12, total: 40 })]), current: { trackId: "x", received: 1 } } }));
    render(<PhoneLibrary />);
    const strip = (await screen.findByText("Downloading", {}, SLOW)).closest(".ph-dl-strip")!;
    expect(strip).toHaveTextContent("12 of 40");
  });
});

describe("offline Home", () => {
  it("has an On this phone shelf of what's downloaded, only while just the phone's music plays", async () => {
    act(() => useDownloads.setState({ status: status([mix()]) }));
    const { rerender } = render(<OnThisPhone />);
    expect(screen.queryByRole("region", { name: "On this phone" })).toBeNull();
    const health = useApp.getState().health;
    act(() => useApp.setState({ health: { ...(health ?? ({} as never)), cachedOnly: true } }));
    rerender(<OnThisPhone />);
    const shelf = screen.getByRole("region", { name: "On this phone" });
    const card = within(shelf).getByRole("button", { name: /Moonlit Hours/ });
    expect(within(card).getByRole("img", { name: "Downloaded" })).toBeInTheDocument();
    fireEvent.click(card);
    expect(route()).toEqual({ name: "mix", id: "mix:moonlit-hours" });
    act(() => useApp.setState({ health }));
  });
});

describe("offline, chosen or not", () => {
  const offline = (kind: "goneOffline" | "noInternet", cached: string[]) =>
    act(() =>
      useApp.setState({
        health: { ...(useApp.getState().health as Health), status: "offline", cachedOnly: true, banner: { kind, message: "" } as Health["banner"] },
        cached: new Set(cached),
      }),
    );
  let health: Health | null;
  beforeEach(() => {
    health = useApp.getState().health;
  });
  afterEach(() => act(() => useApp.setState({ health, cached: null })));

  it("Search lists only the songs that can play, as Songs on this phone", async () => {
    const artist = (await backend.albumPage("1000")).album.artistTitle;
    const res = await backend.search({ query: artist, filter: "all", limit: 20 });
    const [here, gone] = res.songs;
    expect(here && gone).toBeTruthy();
    offline("noInternet", [here.id]);
    useRouter.getState().reset({ name: "search", query: artist });
    render(<PhoneSearch />);
    const section = await screen.findByRole("region", { name: "Songs on this phone" }, SLOW);
    expect(within(section).getByText(here.title, { selector: ".ph-row__title" })).toBeInTheDocument();
    expect(within(section).queryByText(gone.title, { selector: ".ph-row__title" })).toBeNull();
    expect(screen.queryByRole("region", { name: "Results" })).toBeNull();
  });

  it("a dimmed song takes a tap and does nothing", () => {
    offline("noInternet", []);
    let played = 0;
    const track = { id: "t-gone", title: "Gone Song", artistTitle: "Someone", albumTitle: "Somewhere", thumb: null } as never;
    render(
      <ul>
        <SongRow track={track} onPlay={() => played++} />
      </ul>,
    );
    expect(screen.getByRole("listitem")).toHaveClass("is-unplayable");
    fireEvent.click(screen.getByRole("listitem").querySelector(".ph-song__main")!);
    expect(played).toBe(0);
  });

  it("Go offline says Offline under the tabs instead of the line over them; other causes keep their line", () => {
    offline("goneOffline", []);
    const { rerender } = render(
      <>
        <HealthStrip />
        <OfflineCaption />
      </>,
    );
    expect(screen.getByText("Offline")).toBeInTheDocument();
    expect(screen.queryByTestId("phone-offline-strip")).toBeNull();
    offline("noInternet", []);
    rerender(
      <>
        <HealthStrip />
        <OfflineCaption />
      </>,
    );
    expect(screen.queryByText("Offline")).toBeNull();
    expect(screen.getByTestId("phone-offline-strip")).toHaveTextContent("only songs on this phone play");
  });
});

describe("storage on the phone", () => {
  it("splits the drive into other apps, downloads, cache and free, adding up to the whole", () => {
    const GB = 1024 ** 3;
    const parts = storageParts(256 * GB, 100 * GB, 60 * GB, 2 * GB);
    expect(parts.map((p) => [p.label, p.bytes / GB])).toEqual([
      ["Other apps", 94],
      ["Downloads", 60],
      ["Cache", 2],
      ["Free", 100],
    ]);
    expect(parts.reduce((n, p) => n + p.bytes, 0)).toBe(256 * GB);
    // A drive that reads oddly never goes below nothing.
    expect(storageParts(10 * GB, 9 * GB, 5 * GB, 0).every((p) => p.bytes >= 0)).toBe(true);
  });
});

describe("review follow-ups", () => {
  it("a mix whose download fails isn't saved", async () => {
    const spy = vi.spyOn(backend, "downloadsSet").mockRejectedValueOnce({ message: "No room" });
    await setDownload({ kind: "mix", id: "mix:moonlit-hours", title: "Moonlit Hours", trackIds: () => ["1"] }, true);
    expect(isMixSaved("mix:moonlit-hours")).toBe(false);
    spy.mockRestore();
  });

  it("Home's shelf finds downloaded albums even when Library hasn't been opened", async () => {
    const a = (await backend.search({ query: "Slow Ghosts", filter: "all", limit: 20 })).albums.find((x) => x.title === "Slow Ghosts")!;
    useLibraryView.setState({ items: null });
    const done = { state: "downloaded" as const, total: 3, done: 3, failed: 0, missing: 0, progress: 1, bytes: 1 };
    act(() => useDownloads.setState({ status: status([{ kind: "album", id: a.id, title: a.title, ...done }]) }));
    const health = useApp.getState().health;
    act(() => useApp.setState({ health: { ...(health as Health), cachedOnly: true } }));
    render(<OnThisPhone />);
    const shelf = await screen.findByRole("region", { name: "On this phone" }, SLOW);
    expect(within(shelf).getByRole("button", { name: /Slow Ghosts/ })).toBeInTheDocument();
    act(() => useApp.setState({ health }));
  });

  it("storage without the drive's size keeps the plain meter", () => {
    act(() => useDownloads.setState({ status: { ...status([]), usedBytes: 5, limitBytes: 10, freeBytes: 100, totalBytes: null } }));
    render(<StorageNote />);
    expect(screen.getByRole("meter", { name: "Download storage used" })).toBeInTheDocument();
  });
});
