import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ContextMenuHost } from "../components/ContextMenu";
import { Toaster } from "../components/Toast";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS, type LibraryFolders } from "../lib/types";
import { SettingsPage } from "../pages/SettingsPage";
import { FolderDialogs } from "../shell/FolderMenus";
import { Sidebar } from "../shell/Sidebar";
import { useApp } from "../stores/app";
import { folders, loadFolders, onStoreChanged, useFolders, withFolders } from "../stores/folders";
import { useLibraryView, type LibEntry } from "../stores/library";
import { loadPins, usePins } from "../stores/pins";
import { useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";
import { useUi } from "../stores/ui";

const mock = backend as MockBackend;

const SIGNED_IN = {
  state: "signedIn" as const,
  username: "listener",
  needsServerPick: false,
  server: { machineId: "mock-machine-01", name: "Basement Server", owned: true, online: true, connection: "local" as const },
};

beforeEach(async () => {
  mock.syncFile.reset();
  await backend.settingsSet({ ...DEFAULT_SETTINGS });
  useSettings.setState({ settings: null });
  useApp.setState({ auth: SIGNED_IN, health: null, sync: { phase: "idle", done: 0, total: 0 } });
  useToasts.setState({ toasts: [] });
  useFolders.setState({ tree: { folders: [], items: {} }, drill: null, expanded: [], renaming: null, deleting: null });
  usePins.setState({ pins: [] });
  useRouter.getState().reset();
  useUi.setState({ menu: null, sidebarCollapsed: false, sidebarArt: true });
  useLibraryView.setState({ filter: "all", sub: null, sort: "alpha", view: "list", customOrder: [], recents: {} });
  await loadSettings();
});
afterEach(() => vi.restoreAllMocks());

// ---------------------------------------------------------------- the model

const e = (kind: LibEntry["kind"], id: string, more: Partial<LibEntry> = {}): LibEntry => ({
  kind,
  id,
  title: id,
  sortTitle: id,
  creator: null,
  creatorId: null,
  thumb: null,
  smart: false,
  format: null,
  addedAt: null,
  playedAt: null,
  pinned: false,
  ...more,
});

describe("folders in the library list", () => {
  const tree: LibraryFolders = {
    folders: [
      { id: "f2", name: "Inner", parent: "f1", createdAt: 3 },
      { id: "f1", name: "Road trips", parent: null, createdAt: 2 },
      { id: "f0", name: "Gym", parent: null, createdAt: 1 },
    ],
    items: { p1: "f1", p2: "f2", gone: "nowhere" },
  };
  const entries = [e("playlist", "liked", { liked: true, pinned: true }), e("album", "a1"), e("playlist", "p1"), e("playlist", "p2"), e("playlist", "p3"), e("playlist", "gone")];

  it("puts top-level folders under the pins and keeps their playlists out of the top", () => {
    const out = withFolders(entries, tree, { active: true, drill: null, expanded: [] });
    expect(out.map((x) => x.id)).toEqual(["liked", "f1", "f0", "a1", "p3", "gone"]);
    expect(out[1].folder).toMatchObject({ count: 1, depth: 0, nested: false });
    // A playlist placed in a folder that isn't there shows at the top.
    expect(out.some((x) => x.id === "gone")).toBe(true);
  });

  it("drilled in: the folder's folders (open ones followed by their contents) then its playlists", () => {
    const closed = withFolders(entries, tree, { active: true, drill: "f1", expanded: [] });
    expect(closed.map((x) => x.id)).toEqual(["f2", "p1"]);
    expect(closed[0].folder).toMatchObject({ nested: true, expanded: false, count: 1 });
    const open = withFolders(entries, tree, { active: true, drill: "f1", expanded: ["f2"] });
    expect(open.map((x) => [x.id, x.folder?.depth ?? x.depth ?? 0])).toEqual([
      ["f2", 0],
      ["p2", 1],
      ["p1", 0],
    ]);
  });

  it("searching or filtering to albums shows the plain list", () => {
    expect(withFolders(entries, tree, { active: false, drill: "f1", expanded: [] })).toBe(entries);
  });
});

// ---------------------------------------------------------------- the sidebar

async function renderSidebar() {
  // Playlists only, so every row is rendered.
  useLibraryView.setState({ filter: "playlist" });
  await act(() => useLibraryView.getState().loadItems());
  await act(() => loadFolders());
  return render(
    <>
      <Sidebar collapsed={false} />
      <ContextMenuHost />
      <FolderDialogs />
      <Toaster />
    </>,
  );
}

const list = (name = "Your Library items") => screen.getByRole("list", { name });
const titles = (name?: string) => within(list(name)).getAllByRole("listitem").map((r) => r.querySelector(".lib-row__title")?.textContent);

describe("library folders", () => {
  it("creates a folder at once, moves a playlist in, drills in and back", async () => {
    const user = userEvent.setup();
    await renderSidebar();
    const playlist = mock.fx.playlists.find((p) => !p.smart)!;
    await user.click(screen.getByRole("button", { name: "Create playlist or folder" }));
    await user.click(await screen.findByRole("menuitem", { name: "Folder" }));
    // "New Folder", right under Liked Songs, with no prompt.
    await waitFor(() => expect(titles()[1]).toBe("New Folder"));
    const folderRow = within(list()).getAllByRole("listitem")[1];
    expect(within(folderRow).getByText("0 playlists")).toBeInTheDocument();
    expect(screen.queryByRole("dialog")).toBeNull();

    // Move to folder › New Folder, from the playlist's menu.
    const row = within(list()).getAllByRole("listitem").find((r) => r.querySelector(".lib-row__title")?.textContent === playlist.title)!;
    fireEvent.contextMenu(row);
    const menu = await screen.findByRole("menu");
    await user.click(within(menu).getByRole("menuitem", { name: "Move to folder" }));
    await user.click(await screen.findByRole("menuitem", { name: "New Folder" }));
    await waitFor(() => expect(titles()).not.toContain(playlist.title));
    expect(within(within(list()).getAllByRole("listitem")[1]).getByText("1 playlist")).toBeInTheDocument();

    // Opening the folder drills the list into it.
    await user.click(within(list()).getAllByRole("listitem")[1].querySelector<HTMLButtonElement>(".lib-row__open")!);
    expect(screen.getByRole("button", { name: "Back from New Folder" })).toBeInTheDocument();
    expect(titles("New Folder items")).toEqual([playlist.title]);
    await user.click(screen.getByRole("button", { name: "Back from New Folder" }));
    expect(screen.getByRole("button", { name: "Collapse Your Library" })).toBeInTheDocument();
  });

  it("nests folders inside a drilled-in folder and opens them in place", async () => {
    const user = userEvent.setup();
    const outer = await backend.folderCreate("Outer", null);
    const inner = await backend.folderCreate("Inner", outer.id);
    const p = mock.fx.playlists.find((x) => !x.smart)!;
    await backend.playlistFolderSet(p.id, inner.id);
    await renderSidebar();
    act(() => folders.open(outer.id));
    expect(titles("Outer items")).toEqual(["Inner"]);
    const open = within(list("Outer items")).getAllByRole("listitem")[0].querySelector<HTMLButtonElement>(".lib-row__open")!;
    expect(open).toHaveAttribute("aria-expanded", "false");
    await user.click(open);
    expect(open).toHaveAttribute("aria-expanded", "true");
    expect(titles("Outer items")).toEqual(["Inner", p.title]);
    // The header's Create makes a folder inside the open folder.
    await user.click(screen.getByRole("button", { name: "Create in Outer" }));
    await user.click(await screen.findByRole("menuitem", { name: "Folder" }));
    await waitFor(() => expect(titles("Outer items")).toContain("New Folder"));
    const t = await backend.foldersGet();
    expect(t.folders.find((f) => f.name === "New Folder")?.parent).toBe(outer.id);
  });

  it("renames in a field with the name selected, and deleting keeps the playlists", async () => {
    const user = userEvent.setup();
    const f = await backend.folderCreate("Road trips", null);
    const p = mock.fx.playlists.find((x) => !x.smart)!;
    await backend.playlistFolderSet(p.id, f.id);
    await renderSidebar();
    act(() => folders.open(f.id));
    await user.click(screen.getByRole("button", { name: "More options for Road trips" }));
    const menu = await screen.findByRole("menu");
    expect(within(menu).getAllByRole("menuitem").map((m) => m.textContent)).toEqual(["Rename", "Delete", "Create playlist", "Create folder", "Move to folder"]);
    await user.click(within(menu).getByRole("menuitem", { name: "Rename" }));
    const field = await screen.findByRole("textbox", { name: "Folder name" });
    await waitFor(() => expect(field).toHaveValue("Road trips"));
    await waitFor(() => expect((field as HTMLInputElement).selectionEnd).toBe("Road trips".length));
    await user.clear(field);
    await user.type(field, "Summer{Enter}");
    await waitFor(() => expect(screen.getByRole("button", { name: "Back from Summer" })).toBeInTheDocument());

    await user.click(screen.getByRole("button", { name: "More options for Summer" }));
    await user.click(within(await screen.findByRole("menu")).getByRole("menuitem", { name: "Delete" }));
    const dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveTextContent("Your playlists stay in Plex");
    await user.click(within(dialog).getByRole("button", { name: "Delete" }));
    // Back at the top, the playlist with it.
    await waitFor(() => expect(screen.getByRole("button", { name: "Collapse Your Library" })).toBeInTheDocument());
    expect(titles()).toContain(p.title);
    expect((await backend.foldersGet()).folders).toHaveLength(0);
  });

  it("another device's change reloads the folders", async () => {
    await renderSidebar();
    await backend.folderCreate("From the mac", null);
    act(() => onStoreChanged({ namespaces: ["folders"], remote: true }));
    await waitFor(() => expect(titles()).toContain("From the mac"));
  });
});

describe("pins", () => {
  it("pinning shows at once and is kept by the core; an older build's pins move over once", async () => {
    const toggle = vi.spyOn(backend, "pinsToggle");
    act(() => usePins.getState().toggle({ kind: "album", id: "a1", title: "A", subtitle: "Album", thumb: null }));
    expect(usePins.getState().pins).toHaveLength(1);
    expect(toggle).toHaveBeenCalledWith(expect.objectContaining({ kind: "album", id: "a1" }));
    await toggle.mock.results[0].value;

    mock.syncFile.reset();
    localStorage.setItem("platter.pins.v1", JSON.stringify([{ kind: "artist", id: "r1", title: "R", subtitle: "Artist", thumb: null }]));
    await loadPins();
    expect(usePins.getState().pins.map((p) => p.id)).toEqual(["r1"]);
    expect(localStorage.getItem("platter.pins.v1")).toBeNull();
    expect((await backend.pinsGet()).map((p) => p.id)).toEqual(["r1"]);
  });
});

// ---------------------------------------------------------------- settings

const row = (title: string | RegExp) => {
  const el = screen.getByText(title, { selector: ".setting-row__title" }).closest(".setting-row");
  if (!el) throw new Error(`no row ${title}`);
  return within(el as HTMLElement);
};

describe("Settings, Sync", () => {
  it("turning sync on suggests the cloud folders found, then shows the state", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    await user.click(screen.getByRole("switch", { name: "Sync with a folder" }));
    const dialog = await screen.findByRole("dialog", { name: "Choose a folder to sync with" });
    await within(dialog).findByText("OneDrive");
    expect(within(dialog).getByText("Dropbox")).toBeInTheDocument();
    expect(dialog).toHaveTextContent("never holds your Plex sign-in or any keys");
    await user.click(within(dialog).getByRole("button", { name: /OneDrive/ }));
    await waitFor(() => expect(screen.getByRole("switch", { name: "Sync with a folder" })).toHaveAttribute("aria-checked", "true"));
    await waitFor(() => expect(row("Sync with a folder").getByText(/Synced just now with Studio Mac/)).toBeInTheDocument());
    expect(row("Sync folder").getByText(/OneDrive\\Platter\\sync-/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Sync now" })).toBeEnabled();
  });

  it("app states: folder missing, unreadable file (start a new one), and a newer file", async () => {
    const user = userEvent.setup();
    await backend.syncFileUseFolder("D:\\Cloud");
    await loadSettings();
    render(<SettingsPage />);
    act(() => mock.syncFile.setFileState("folderMissing"));
    await waitFor(() => expect(row("Sync with a folder").getByText(/isn't there. Changes are kept on this computer/)).toBeInTheDocument());
    act(() => mock.syncFile.setFileState("unreadable"));
    await waitFor(() => expect(row("Sync with a folder").getByText(/can't be read/)).toBeInTheDocument());
    await user.click(screen.getByRole("button", { name: "Start a new file" }));
    await waitFor(() => expect(row("Sync with a folder").getByText(/Synced just now/)).toBeInTheDocument());
    act(() => mock.syncFile.setFileState("newer"));
    await waitFor(() => expect(row("Sync with a folder").getByText(/newer Platter/)).toBeInTheDocument());
  });

  it("what to sync is saved per device", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    await user.click(screen.getByRole("switch", { name: "Lyrics you wrote" }));
    await waitFor(async () => expect((await backend.settingsGet()).syncLyrics).toBe(false));
  });
});

describe("copying keys", () => {
  it("shows a code and its QR code, and says what arrived", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    await user.click(row("Copy keys from another device").getByRole("button", { name: "Show a code" }));
    const dialog = await screen.findByRole("dialog", { name: "Copy keys from another device" });
    const code = await within(dialog).findByLabelText(/^Code \d \d \d \d \d \d$/);
    expect(code.textContent).toMatch(/^\d{3} \d{3}$/);
    expect(within(dialog).getByRole("img", { name: "QR code for the code" })).toBeInTheDocument();
    expect(within(dialog).getByText(/Works for [45]:\d\d/)).toBeInTheDocument();
    act(() => mock.syncFile.keysArrive(["lastfm", "imgur"]));
    await within(dialog).findByText(/Got 2 keys from Studio Mac: Last.fm API key, Imgur client ID/);
    await user.click(within(dialog).getByRole("button", { name: "Done" }));
  });

  it("a code that ran out offers a new one", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    await user.click(row("Copy keys from another device").getByRole("button", { name: "Show a code" }));
    const dialog = await screen.findByRole("dialog", { name: "Copy keys from another device" });
    await within(dialog).findByLabelText(/^Code /);
    act(() => mock.syncFile.expireCode());
    await within(dialog).findByText(/That code ran out/);
    await user.click(within(dialog).getByRole("button", { name: "New code" }));
    await within(dialog).findByLabelText(/^Code /);
  });

  it("sends the ticked keys with the right code and explains a wrong one", async () => {
    const user = userEvent.setup();
    const send = vi.spyOn(backend, "keysSend");
    render(<SettingsPage />);
    await user.click(row("Send keys to a device").getByRole("button", { name: "Enter a code" }));
    const dialog = await screen.findByRole("dialog", { name: "Send keys to a device" });
    const lastfm = await within(dialog).findByRole("checkbox", { name: "Last.fm API key" });
    expect(lastfm).toBeChecked();
    expect(within(dialog).queryByRole("checkbox", { name: "Fanart.tv API key" })).toBeNull();
    await user.click(within(dialog).getByRole("checkbox", { name: "catbox.moe uploads turned on" }));
    const field = within(dialog).getByRole("textbox", { name: "Code" });
    const sendButton = within(dialog).getByRole("button", { name: "Send" });
    expect(sendButton).toBeDisabled();
    await user.type(field, "111 111");
    await user.click(sendButton);
    await within(dialog).findByRole("alert");
    expect(within(dialog).getByRole("alert")).toHaveTextContent("That code doesn't match");
    await user.clear(field);
    await user.type(field, "246810");
    await user.click(sendButton);
    await within(dialog).findByText("Sent 3 keys to Studio Mac.");
    expect(send).toHaveBeenLastCalledWith("246810", ["listenbrainz", "lastfm", "imgur"]);
  });

  it("app state: no device showing a code", async () => {
    const user = userEvent.setup();
    mock.syncFile.noDevice = true;
    render(<SettingsPage />);
    await user.click(row("Send keys to a device").getByRole("button", { name: "Enter a code" }));
    const dialog = await screen.findByRole("dialog", { name: "Send keys to a device" });
    await within(dialog).findByRole("checkbox", { name: "Last.fm API key" });
    await user.type(within(dialog).getByRole("textbox", { name: "Code" }), "246810");
    await user.click(within(dialog).getByRole("button", { name: "Send" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("No device is showing a code");
  });
});
