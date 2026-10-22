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
  useLibraryView.getState().setFilter("all");
  useLibraryView.getState().setDownloaded(null);
  useRouter.getState().reset({ name: "home" });
});
afterEach(() => {
  usePhoneForm.setState({ phone: false });
  resetDownloads();
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

    fireEvent.click(within(show).getByRole("button", { name: "Downloaded" }));
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
