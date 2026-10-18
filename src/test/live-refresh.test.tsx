/**
 * Live library updates: an album the server adds while the app is open
 * shows up on Home (Recently added), in the library list, on its artist's
 * page and in What's New without a manual refresh, when the core says
 * `library://changed`.
 */
import { render, renderHook, screen, waitFor, within } from "@testing-library/react";
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import type { Album, Artist } from "../lib/types";
import { DEFAULT_SETTINGS } from "../lib/types";
import { ArtistPage } from "../pages/ArtistPage";
import { HomePage } from "../pages/HomePage";
import { WhatsNewPage } from "../pages/WhatsNewPage";
import { useLibraryLoader } from "../shell/useLibrary";
import { ONLINE_EXTRAS } from "./onlineExtras";
import { connectApp, useApp } from "../stores/app";
import { useLibraryView } from "../stores/library";
import { useRouter } from "../stores/router";
import { useSettings } from "../stores/settings";

const mock = backend as MockBackend;

let disconnect: () => void;
beforeAll(() => {
  disconnect = connectApp(mock);
});
afterAll(() => disconnect());
afterEach(() => vi.restoreAllMocks());
// The usual Home, with its shelves (not the record-led one).
const USUAL_HOME = { ...ONLINE_EXTRAS, homeRecordsFirst: false };

beforeEach(async () => {
  useRouter.getState().reset();
  await mock.settingsSet({ ...DEFAULT_SETTINGS, ...USUAL_HOME });
  useSettings.setState({ settings: { ...DEFAULT_SETTINGS, ...USUAL_HOME } });
  Element.prototype.scrollTo = vi.fn();
});

let n = 0;
function fresh(): { album: Album; artist: Artist } {
  n += 1;
  const artist: Artist = { id: `live-ar${n}`, title: `Brand New Band ${n}`, sortTitle: `Brand New Band ${n}`, thumb: null, art: null, viewCount: 0 };
  const album: Album = {
    id: `live-al${n}`,
    title: `Fresh From The Scan ${n}`,
    sortTitle: `Fresh From The Scan ${n}`,
    artistId: artist.id,
    artistTitle: artist.title,
    year: 2026,
    thumb: null,
    format: "album",
    subformats: [],
    section: "albums",
    genres: [],
    trackCount: 0,
    addedAt: Math.floor(Date.now() / 1000) + 1000 * n,
    viewCount: 0,
    lastViewedAt: null,
    tint: null,
  };
  return { album, artist };
}

describe("live library updates", () => {
  it("Home's Recently added shows a new album when the library changes", async () => {
    render(<HomePage />);
    await screen.findByRole("heading", { name: /Recently added/i });
    const { album, artist } = fresh();
    expect(screen.queryByText(album.title)).not.toBeInTheDocument();
    const before = useApp.getState().libraryVersion;
    mock.libraryAdded(album, artist);
    await waitFor(() => expect(useApp.getState().libraryVersion).toBe(before + 1));
    expect((await screen.findAllByText(album.title)).length).toBeGreaterThan(0);
  });

  it("the library list loads again and has the new album and artist", async () => {
    renderHook(() => useLibraryLoader());
    await waitFor(() => expect(useLibraryView.getState().items).not.toBeNull());
    const { album, artist } = fresh();
    mock.libraryAdded(album, artist);
    await waitFor(() => expect(useLibraryView.getState().items!.some((i) => i.id === album.id)).toBe(true));
  });

  it("an open artist page picks up their new album", async () => {
    const { album, artist } = fresh();
    mock.libraryAdded({ ...album, id: `${album.id}-first`, title: "Their First" }, artist);
    render(<ArtistPage id={artist.id} />);
    expect(await screen.findByRole("heading", { name: artist.title })).toBeInTheDocument();
    expect(screen.queryByText(album.title)).not.toBeInTheDocument();
    mock.libraryAdded(album);
    expect((await screen.findAllByText(album.title)).length).toBeGreaterThan(0);
  });

  it("What's New asks again when the library changes", async () => {
    const spy = vi.spyOn(backend, "whatsNew");
    render(<WhatsNewPage />);
    await waitFor(() => expect(spy).toHaveBeenCalledTimes(1));
    const { album, artist } = fresh();
    mock.libraryAdded(album, artist);
    await waitFor(() => expect(spy).toHaveBeenCalledTimes(2));
  });

  it("What's New keeps what was new in New while the page stays open", async () => {
    const a = fresh().album;
    const b = fresh().album;
    vi.spyOn(backend, "whatsNewSeen").mockResolvedValue(undefined);
    vi.spyOn(backend, "whatsNew")
      .mockResolvedValueOnce({ following: true, unseen: 1, items: [{ album: a, new: true }] })
      .mockResolvedValue({ following: true, unseen: 1, items: [{ album: b, new: true }, { album: a, new: false }] });
    render(<WhatsNewPage />);
    const newGroup = () => screen.getByRole("heading", { name: "New" }).closest("section")!;
    await waitFor(() => expect(within(newGroup()).getByText(a.title)).toBeInTheDocument());
    mock.libraryAdded(b);
    await waitFor(() => expect(within(newGroup()).getByText(b.title)).toBeInTheDocument());
    expect(within(newGroup()).getByText(a.title)).toBeInTheDocument();
  });
});
