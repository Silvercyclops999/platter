/**
 * The physical collection in the UI: the "On vinyl" badge and "I own this"
 * on album pages and menus, the copy editor, the library's On the shelf
 * chip, the Stats page's shelf, Missing from the shelf, the Discogs token
 * and import (a faked Discogs: refused, busy and unreachable too) with
 * "On the shelf, not in your library" and Request, and the settings.
 */
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ContextMenuHost } from "../components/ContextMenu";
import { PhysicalDialogHost } from "../components/PhysicalDialog";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { MockCatalogue } from "../lib/mock/catalogue";
import { MOCK_DISCOGS_USER } from "../lib/mock/physical";
import { DEFAULT_SETTINGS, type Album } from "../lib/types";
import { AlbumPage } from "../pages/AlbumPage";
import { LibraryPage } from "../pages/LibraryPage";
import { SettingsPage } from "../pages/SettingsPage";
import { ShelfPage, ShelfStatsSection, formatLine } from "../pages/ShelfPage";
import { useApp } from "../stores/app";
import { chipRow, libraryEntries, type LibEntry } from "../stores/library";
import { useLibraryView } from "../stores/library";
import { shelfLabel, usePhysical } from "../stores/physical";
import { useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";

const mock = backend as MockBackend;
const SLOW = { timeout: 4000 };
const route = () => useRouter.getState().entries[useRouter.getState().index].route;

async function setUp(patch: Record<string, unknown> = {}, lidarr: "ready" | null = null) {
  if (lidarr) mock.lidarr.reset(lidarr);
  await mock.settingsSet({ ...DEFAULT_SETTINGS, lidarrUrl: lidarr ? "http://10.0.0.5:8686" : "", ...patch });
  useSettings.setState({ settings: null });
  await loadSettings();
}

/** Albums the mock library plays most (the seeded marks are on the 1st, 3rd and 6th). */
const played = (): Album[] => [...mock.fx.albums].filter((a) => a.format !== null).sort((a, b) => b.viewCount - a.viewCount);

beforeEach(async () => {
  useApp.setState({ auth: { state: "signedIn", username: "listener", needsServerPick: false } });
  useToasts.setState({ toasts: [] });
  useRouter.getState().reset();
  mock.physical.reset();
  usePhysical.setState({ owned: null, discogs: null });
  await usePhysical.getState().load();
  await usePhysical.getState().loadDiscogs();
  await setUp();
});

describe("helpers", () => {
  it("says what an album is owned on", () => {
    expect(shelfLabel(["vinyl"])).toBe("On vinyl");
    expect(shelfLabel(["cd", "vinyl"])).toBe("On vinyl and CD");
    expect(shelfLabel(["cassette", "cd", "vinyl"])).toBe("On vinyl, CD and cassette");
    expect(shelfLabel(["other"])).toBe("On another format");
    expect(shelfLabel([])).toBe("");
    expect(
      formatLine({ albums: 2, copies: 3, byFormat: [{ format: "vinyl", albums: 1, copies: 2 }, { format: "cd", albums: 1, copies: 1 }], mostPlayed: [], missing: null }),
    ).toBe("2 on vinyl · 1 on CD");
  });

  it("offers the On the shelf chip under Albums only when something's owned", () => {
    expect(chipRow("album", null)).toEqual([{ kind: "clear" }, { kind: "type", type: "album", selected: true }]);
    expect(chipRow("album", null, undefined, true).map((c) => (c.kind === "sub" ? c.sub : c.kind))).toEqual(["clear", "type", "shelf"]);
    const e = (id: string, owned: boolean): LibEntry => ({
      kind: "album",
      id,
      title: id,
      sortTitle: id,
      creator: null,
      creatorId: null,
      thumb: null,
      smart: false,
      format: "album",
      addedAt: null,
      playedAt: null,
      pinned: false,
      ...(owned ? { owned: true } : {}),
    });
    const shown = libraryEntries([e("a", true), e("b", false)], { filter: "album", sub: "shelf", sort: "alpha", query: "", recents: {} });
    expect(shown.map((x) => x.id)).toEqual(["a"]);
  });
});

describe("album page", () => {
  it("shows the badge for an owned album and edits what's owned from the menu", async () => {
    const top = played()[0];
    render(
      <>
        <AlbumPage id={top.id} />
        <PhysicalDialogHost />
        <ContextMenuHost />
      </>,
    );
    expect(await screen.findByText("On vinyl", {}, SLOW)).toBeInTheDocument();
    await userEvent.click(await screen.findByRole("button", { name: `More options for ${top.title}` }));
    await userEvent.click(await screen.findByRole("menuitem", { name: "Edit what you own" }));
    const dialog = await screen.findByRole("dialog", { name: `What you own of ${top.title}` });
    expect(within(dialog).getByLabelText("Pressing")).toHaveValue("LP, Album, Reissue");
    // Add a CD.
    await userEvent.click(within(dialog).getByRole("button", { name: "Add another copy" }));
    const second = within(dialog).getByRole("radiogroup", { name: "Format of copy 2" });
    await userEvent.click(within(second).getByRole("radio", { name: "CD" }));
    const notes = within(dialog).getAllByLabelText("Notes")[1];
    await userEvent.type(notes, "Japanese edition");
    await userEvent.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(await screen.findByText("On vinyl and CD")).toBeInTheDocument();
    const mark = mock.physical.marks.get(top.id)!;
    expect(mark.copies.map((c) => c.format)).toEqual(["vinyl", "cd"]);
    expect(mark.copies[1].notes).toBe("Japanese edition");
  });

  it("marks an album owned with I own this, and I don't own it takes it off", async () => {
    const a = played()[1];
    render(
      <>
        <AlbumPage id={a.id} />
        <PhysicalDialogHost />
        <ContextMenuHost />
      </>,
    );
    await screen.findByRole("heading", { level: 1, name: a.title }, SLOW);
    expect(screen.queryByText(/^On (vinyl|CD|cassette)/)).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: `More options for ${a.title}` }));
    await userEvent.click(await screen.findByRole("menuitem", { name: "I own this" }));
    let dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("radio", { name: "Cassette" }));
    await userEvent.type(within(dialog).getByLabelText("Condition"), "Very good");
    await userEvent.click(within(dialog).getByRole("button", { name: "Save" }));
    expect(await screen.findByText("On cassette")).toBeInTheDocument();
    expect(mock.physical.marks.get(a.id)?.copies[0]).toMatchObject({ format: "cassette", condition: "Very good" });

    await userEvent.click(screen.getByRole("button", { name: `More options for ${a.title}` }));
    await userEvent.click(await screen.findByRole("menuitem", { name: "Edit what you own" }));
    dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "I don't own it" }));
    await waitFor(() => expect(screen.queryByText("On cassette")).toBeNull());
    expect(mock.physical.marks.has(a.id)).toBe(false);
  });

  it("hides the badge and the menu item while Show physical collection is off", async () => {
    await setUp({ showPhysicalCollection: false });
    const top = played()[0];
    render(
      <>
        <AlbumPage id={top.id} />
        <ContextMenuHost />
      </>,
    );
    await screen.findByRole("heading", { level: 1, name: top.title }, SLOW);
    expect(screen.queryByText("On vinyl")).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: `More options for ${top.title}` }));
    await screen.findByRole("menuitem", { name: "Add to queue" });
    expect(screen.queryByRole("menuitem", { name: /I own this|what you own/ })).toBeNull();
  });
});

describe("library", () => {
  it("filters albums to the ones on the shelf", async () => {
    useLibraryView.getState().setFilter("album");
    render(<LibraryPage />);
    const chip = await screen.findByRole("button", { name: "On the shelf" }, SLOW);
    await userEvent.click(chip);
    expect(chip).toHaveAttribute("aria-pressed", "true");
    const owned = new Set(mock.physical.marks.keys());
    await waitFor(() => {
      const titles = mock.fx.albums.filter((a) => owned.has(a.id)).map((a) => a.title);
      for (const t of titles) expect(screen.getAllByText(t).length).toBeGreaterThan(0);
      expect(screen.queryByText(played()[1].title)).toBeNull();
    });
    useLibraryView.getState().setFilter("all");
  });

  it("has no shelf chip while the setting is off", async () => {
    await setUp({ showPhysicalCollection: false });
    useLibraryView.getState().setFilter("album");
    render(<LibraryPage />);
    await screen.findByRole("button", { name: "Albums" }, SLOW);
    expect(screen.queryByRole("button", { name: "On the shelf" })).toBeNull();
    useLibraryView.getState().setFilter("all");
  });
});

describe("stats", () => {
  it("the shelf page counts the shelf by format, with the owned albums played most", async () => {
    render(<ShelfPage />);
    const section = await screen.findByRole("region", { name: "Your shelf in numbers" }, SLOW);
    expect(within(section).getByText("Vinyl").closest("div")).toHaveTextContent("Vinyl2");
    expect(within(section).getByText("Cassette")).toBeInTheDocument();
    const most = within(section).getByRole("list", { name: "Owned albums you play most" });
    expect(within(most).getAllByRole("listitem")[0]).toHaveTextContent(played()[0].title);
    // The page's own list is the only Missing from the shelf.
    expect(within(section).queryByText("Missing from the shelf")).toBeNull();
  });

  it("lists what's missing on the shelf page once that's turned on, once", async () => {
    await setUp({ shelfMissingList: true });
    render(<ShelfPage />);
    const list = await screen.findByRole("list", { name: "Missing from the shelf" }, SLOW);
    expect(within(list).getAllByRole("listitem")[0]).toHaveTextContent(played()[1].title);
    await screen.findByRole("region", { name: "Your shelf in numbers" }, SLOW);
    expect(screen.getAllByRole("list", { name: "Missing from the shelf" })).toHaveLength(1);
  });

  it("is hidden with nothing owned, and while Show physical collection is off", async () => {
    render(<ShelfStatsSection stats={{ albums: 0, copies: 0, byFormat: [], mostPlayed: [], missing: null }} />);
    expect(screen.queryByRole("region", { name: "Your shelf in numbers" })).toBeNull();
    await setUp({ showPhysicalCollection: false });
    const spy = vi.spyOn(mock, "physicalStats");
    render(<ShelfPage />);
    expect(await screen.findByText("Your physical collection is hidden")).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "Your shelf in numbers" })).toBeNull();
    expect(spy).not.toHaveBeenCalled();
    spy.mockRestore();
  });
});

describe("Discogs", () => {
  const typeToken = async (token: string) => {
    render(<SettingsPage />);
    const field = await screen.findByLabelText("Discogs personal access token", {}, SLOW);
    await userEvent.type(field, token);
    await userEvent.click(within(field.closest("form")!).getByRole("button", { name: "Save token" }));
  };

  it("saves a token Discogs accepts, without ever showing it again", async () => {
    await typeToken("fake-token-123");
    expect(await screen.findByText(`Saved for ${MOCK_DISCOGS_USER}.`, {}, SLOW)).toBeInTheDocument();
    expect(screen.queryByLabelText("Discogs personal access token")).toBeNull();
    expect(document.body.textContent).not.toContain("fake-token-123");
    expect(mock.physical.checked).toEqual(["fake-token-123"]);
    await userEvent.click(screen.getByRole("button", { name: "Remove token" }));
    expect(await screen.findByLabelText("Discogs personal access token")).toBeInTheDocument();
  });

  it("says when Discogs refuses the token, and keeps nothing", async () => {
    mock.physical.refuse = true;
    await typeToken("wrong");
    await waitFor(() => expect(useToasts.getState().toasts.map((t) => t.message)).toContain("Discogs didn't accept that token. Copy a new one from your Discogs developer settings."));
    expect(mock.physical.status().saved).toBe(false);
  });

  it("says when Discogs is busy or out of reach", async () => {
    mock.physical.fail = "network";
    await typeToken("any");
    await waitFor(() => expect(useToasts.getState().toasts.map((t) => t.message)).toContain("Couldn't reach Discogs. Check your connection."));
    mock.physical.fail = null;
    mock.physical.setToken("any");
    mock.physical.fail = "busy";
    await expect(backend.discogsImport()).rejects.toMatchObject({ message: "Discogs is busy. Try again in a minute." });
  });

  it("imports the collection: marks what the library has and lists the rest with Request and store links", async () => {
    await setUp({}, "ready");
    mock.physical.setToken("fake-token");
    await usePhysical.getState().loadDiscogs();
    render(
      <>
        <ShelfPage />
        <PhysicalDialogHost />
        <ContextMenuHost />
      </>,
    );
    await userEvent.click(await screen.findByRole("button", { name: "Import collection" }, SLOW));
    const section = await screen.findByRole("region", { name: "On the shelf, not in your library" }, SLOW);
    const rows = within(section).getAllByRole("listitem");
    expect(rows.map((r) => r.querySelector(".shelf-only__title")?.textContent)).toEqual(["Harbour Static", "Copper Seasons", "Basement Tapes Vol. 2", "Saltmarsh"]);
    expect(within(section).getByText("Data provided by Discogs.")).toBeInTheDocument();
    // The library's records are marked now (the 2nd most played was new).
    await waitFor(() => expect(mock.physical.marks.has(played()[1].id)).toBe(true));
    expect(screen.getByText(/Discogs collection of shelfkeeper: 7 records, 3 in your library/)).toBeInTheDocument();

    // Store links open through the core.
    const open = vi.spyOn(mock, "importOpenLink");
    await userEvent.click(within(rows[0]).getByRole("button", { name: "Search Bandcamp for Harbour Static" }));
    expect(open).toHaveBeenCalledWith(expect.stringContaining("bandcamp.com/search?q=Cold%20Coast%20Harbour%20Static"));
    open.mockRestore();

    // Request finds the album on MusicBrainz and goes through Lidarr's flow.
    const preview = vi.spyOn(mock, "lidarrRequestPreview");
    await userEvent.click(within(rows[0]).getByRole("button", { name: "Request Harbour Static" }));
    await screen.findByRole("dialog", {}, SLOW);
    // The open dialog asks for the preview from an effect, which can run after it shows.
    await waitFor(() => expect(preview).toHaveBeenCalledWith(expect.objectContaining({ releaseGroupId: MockCatalogue.albumId(12), artist: "Cold Coast" })));
    preview.mockRestore();
  });

  it("says when MusicBrainz doesn't know a record", async () => {
    await setUp({}, "ready");
    mock.physical.setToken("fake-token");
    mock.physical.import();
    await usePhysical.getState().loadDiscogs();
    render(<ShelfPage />);
    await userEvent.click(await screen.findByRole("button", { name: "Request Basement Tapes Vol. 2" }, SLOW));
    await waitFor(() => expect(useToasts.getState().toasts.map((t) => t.message)).toContain("MusicBrainz doesn't know Basement Tapes Vol. 2 yet, so it can't be requested"));
  });

  it("points to Settings when there's no token", async () => {
    render(<ShelfPage />);
    expect(await screen.findByText(/add your personal token in Settings/, {}, SLOW)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Open Settings" }));
    expect(route()).toEqual({ name: "settings" });
  });
});

describe("settings", () => {
  it("has the rows, with the defaults", async () => {
    render(<SettingsPage />);
    for (const k of ["showPhysicalCollection", "shelfMissingList", "shelfOpen", "discogsToken", "discogsImport"]) {
      expect(await screen.findByText((_, el) => el?.getAttribute("data-setting") === k, {}, SLOW)).toBeInTheDocument();
    }
    expect(DEFAULT_SETTINGS.showPhysicalCollection).toBe(true);
    expect(DEFAULT_SETTINGS.shelfMissingList).toBe(false);
    expect(screen.getByLabelText("Discogs personal access token")).toHaveValue("");
  });

  it("turning off Show physical collection hides the shelf page", async () => {
    await setUp({ showPhysicalCollection: false });
    render(<ShelfPage />);
    expect(screen.getByRole("heading", { name: "Your physical collection is hidden" })).toBeInTheDocument();
  });
});
