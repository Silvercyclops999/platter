/**
 * Edit in Plex: the menu shows only where this account can edit, the cover
 * picker, fix match with the unreleased warning, edit details with locks,
 * merge, recent changes and undo, the settings, and failures (no permission,
 * server offline, image too large).
 */
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { ContextMenuHost } from "../components/ContextMenu";
import { PlexEditDialogs } from "../components/PlexEditDialogs";
import { Toaster } from "../components/Toast";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS, type Album, type Artist } from "../lib/types";
import { AlbumCard, ArtistCard } from "../pages/cards";
import { PlexEditSettings, plexEditMatches } from "../pages/PlexEditSettings";
import { connectApp } from "../stores/app";
import { canEditId, reloadEditAccess, usePlexEdit } from "../stores/plexEdit";
import { useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";
import { useUi } from "../stores/ui";

const mock = backend as MockBackend;

let disconnect: () => void;
beforeAll(() => {
  disconnect = connectApp(mock);
});
afterAll(() => disconnect());

beforeEach(async () => {
  mock.plexEdit.shared = false;
  useToasts.setState({ toasts: [] });
  useUi.setState({ menu: null });
  usePlexEdit.setState({ dialog: null });
  await mock.settingsSet({ ...DEFAULT_SETTINGS });
  useSettings.setState({ settings: { ...DEFAULT_SETTINGS } });
  await reloadEditAccess();
});

const demo = (): Album => mock.fx.albums.find((a) => a.subformats.includes("demo"))!;
const released = (): Album => mock.fx.albums.find((a) => a.format === "album" && !a.subformats.length)!;
const toasts = () => useToasts.getState().toasts;

function Hosts() {
  return (
    <>
      <ContextMenuHost />
      <PlexEditDialogs />
      <Toaster />
    </>
  );
}

async function openEditMenu(user: ReturnType<typeof userEvent.setup>, card: "album" | "artist", item: Album | Artist) {
  render(
    <>
      {card === "album" ? <AlbumCard album={item as Album} /> : <ArtistCard artist={item as Artist} />}
      <Hosts />
    </>,
  );
  fireEvent.contextMenu(screen.getByText(item.title), { clientX: 10, clientY: 10 });
  await user.click(await screen.findByRole("menuitem", { name: "Edit" }));
  return screen.findByRole("menu", { name: "Edit" });
}

describe("who sees it", () => {
  it("tagged ids follow their server; untagged ones the connected server", () => {
    const a = { show: true, servers: ["0a1b2c3d"], connected: false, lockFields: true, matchUnreleased: true };
    expect(canEditId(a, true, "0a1b2c3d:5")).toBe(true);
    expect(canEditId(a, true, "99999999:5")).toBe(false);
    expect(canEditId(a, true, "5")).toBe(false);
    expect(canEditId({ ...a, connected: true }, true, "5")).toBe(true);
    expect(canEditId(a, false, "0a1b2c3d:5")).toBe(false);
    expect(canEditId(null, true, "5")).toBe(false);
  });

  it("owners get Edit in Plex on album cards", async () => {
    const user = userEvent.setup();
    const sub = await openEditMenu(user, "album", released());
    const names = within(sub).getAllByRole("menuitem").map((m) => m.textContent);
    expect(names).toEqual(["Change cover…", "Fix match…", "Unmatch", "Edit details…", "Refresh details", "Recent changes…"]);
  });

  it("artists also get background, merge and split", async () => {
    const user = userEvent.setup();
    const sub = await openEditMenu(user, "artist", mock.fx.artists[0]);
    const names = within(sub).getAllByRole("menuitem").map((m) => m.textContent);
    expect(names).toContain("Change photo…");
    expect(names).toContain("Change background…");
    expect(names).toContain("Merge with…");
    expect(names).toContain("Split…");
  });

  it("shared users never see it", async () => {
    mock.plexEdit.shared = true;
    await reloadEditAccess();
    render(
      <>
        <AlbumCard album={released()} />
        <Hosts />
      </>,
    );
    fireEvent.contextMenu(screen.getByText(released().title), { clientX: 10, clientY: 10 });
    await screen.findByRole("menuitem", { name: "Play" });
    expect(screen.queryByRole("menuitem", { name: "Edit" })).not.toBeInTheDocument();
  });

  it("turned off in Settings, it's hidden", async () => {
    act(() => useSettings.setState({ settings: { ...DEFAULT_SETTINGS, plexEditShow: false } }));
    render(
      <>
        <AlbumCard album={released()} />
        <Hosts />
      </>,
    );
    fireEvent.contextMenu(screen.getByText(released().title), { clientX: 10, clientY: 10 });
    await screen.findByRole("menuitem", { name: "Play" });
    expect(screen.queryByRole("menuitem", { name: "Edit" })).not.toBeInTheDocument();
  });

  it("Fix match hides on unreleased albums when that setting is off; Unmatch stays", async () => {
    await mock.settingsSet({ plexEditMatchUnreleased: false });
    act(() => useSettings.setState({ settings: { ...DEFAULT_SETTINGS, plexEditMatchUnreleased: false } }));
    const user = userEvent.setup();
    const sub = await openEditMenu(user, "album", demo());
    await waitFor(() => expect(within(sub).queryByRole("menuitem", { name: "Fix match…" })).not.toBeInTheDocument());
    expect(within(sub).getByRole("menuitem", { name: "Unmatch" })).toBeInTheDocument();
  });
});

describe("change cover", () => {
  it("picks a suggestion, saves, shows at once, and Undo puts the old one back", async () => {
    const user = userEvent.setup();
    const album = released();
    const before = album.thumb;
    const sub = await openEditMenu(user, "album", album);
    await user.click(within(sub).getByRole("menuitem", { name: "Change cover…" }));
    const dlg = await screen.findByRole("dialog", { name: "Change cover" });
    const tiles = await within(dlg).findAllByRole("radio");
    expect(tiles[0]).toHaveTextContent("Upload an image");
    expect(tiles[1]).toHaveTextContent("In use now");
    expect(within(dlg).getByRole("button", { name: "Save" })).toBeDisabled();
    await user.click(tiles[2]);
    await user.click(within(dlg).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(album.thumb).not.toBe(before);
    const t = toasts().find((x) => x.message === "Changed the cover")!;
    expect(t.action?.label).toBe("Undo");
    act(() => t.action!.run());
    await waitFor(() => expect(album.thumb).toBe(before));
  });

  it("uploads a file", async () => {
    const user = userEvent.setup();
    const album = released();
    const sub = await openEditMenu(user, "album", album);
    await user.click(within(sub).getByRole("menuitem", { name: "Change cover…" }));
    const dlg = await screen.findByRole("dialog", { name: "Change cover" });
    const file = new File([new Uint8Array([0xff, 0xd8, 0xff, 1])], "c.jpg", { type: "image/jpeg" });
    await user.upload(within(dlg).getByTestId("pe-file"), file);
    await waitFor(() => expect(within(dlg).getByRole("button", { name: "Save" })).toBeEnabled());
    await user.click(within(dlg).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(album.thumb).toMatch(/\/upload\//));
  });

  it("an image too large for the server says so and keeps the dialog open", async () => {
    const user = userEvent.setup();
    const sub = await openEditMenu(user, "album", released());
    await user.click(within(sub).getByRole("menuitem", { name: "Change cover…" }));
    const dlg = await screen.findByRole("dialog", { name: "Change cover" });
    await user.type(within(dlg).getByLabelText("Use an image link"), "https://example.com/huge.jpg");
    mock.plexEdit.failNext("tooLarge");
    await user.click(within(dlg).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(toasts().map((t) => t.message)).toContain("Couldn't change the cover: That image is too large for the server."));
    expect(screen.getByRole("dialog", { name: "Change cover" })).toBeInTheDocument();
  });
});

describe("fix match", () => {
  it("warns on unreleased albums and offers Unmatch instead", async () => {
    usePlexEdit.getState().open("match", { id: demo().id, kind: "album", title: demo().title, thumb: demo().thumb });
    render(<Hosts />);
    const dlg = await screen.findByRole("dialog", { name: "Fix match" });
    expect(await within(dlg).findByText("This album counts as unreleased")).toBeInTheDocument();
    const user = userEvent.setup();
    await user.click(within(dlg).getByRole("button", { name: "Unmatch instead" }));
    await waitFor(() => expect(toasts().map((t) => t.message)).toContain("Unmatched"));
  });

  it("searches with the title and year prefilled and matches the pick", async () => {
    const album = released();
    usePlexEdit.getState().open("match", { id: album.id, kind: "album", title: album.title, thumb: album.thumb });
    render(<Hosts />);
    const dlg = await screen.findByRole("dialog", { name: "Fix match" });
    expect(within(dlg).queryByText("This album counts as unreleased")).not.toBeInTheDocument();
    expect(within(dlg).getByLabelText("Album title")).toHaveValue(album.title);
    const options = await within(dlg).findAllByRole("radio");
    expect(options[0]).toHaveTextContent("98% match");
    const user = userEvent.setup();
    await user.click(options[1]);
    await user.click(within(dlg).getByRole("button", { name: "Match" }));
    await waitFor(() => expect(toasts().map((t) => t.message)).toContain(`Matched to ${album.title} (Deluxe Edition)`));
  });
});

describe("edit details", () => {
  it("saves changed fields, locks them, and recent changes can undo", async () => {
    const album = released();
    const old = album.title;
    usePlexEdit.getState().open("details", { id: album.id, kind: "album", title: album.title, thumb: album.thumb });
    const { unmount } = render(<Hosts />);
    const dlg = await screen.findByRole("dialog", { name: "Edit details" });
    expect(within(dlg).getByText(/album artist and song artists come from the files' tags/)).toBeInTheDocument();
    const user = userEvent.setup();
    const title = within(dlg).getByLabelText("Title");
    await user.clear(title);
    await user.type(title, "Renamed");
    await user.click(within(dlg).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(album.title).toBe("Renamed"));
    expect((await backend.plexEditItem(album.id)).locked).toContain("title");
    unmount();

    usePlexEdit.getState().open("history", { id: album.id, kind: "album", title: album.title, thumb: album.thumb });
    render(<Hosts />);
    const h = await screen.findByRole("dialog", { name: "Recent changes" });
    expect(await within(h).findByText("Edited the title")).toBeInTheDocument();
    await user.click(within(h).getAllByRole("button", { name: "Undo" })[0]);
    await waitFor(() => expect(album.title).toBe(old));
    expect(await within(h).findAllByText("Undone")).not.toHaveLength(0);
  });

  it("an empty title isn't sent", async () => {
    const album = released();
    usePlexEdit.getState().open("details", { id: album.id, kind: "album", title: album.title, thumb: album.thumb });
    render(<Hosts />);
    const dlg = await screen.findByRole("dialog", { name: "Edit details" });
    const user = userEvent.setup();
    await user.clear(within(dlg).getByLabelText("Title"));
    await user.click(within(dlg).getByRole("button", { name: "Save" }));
    expect(within(dlg).getByRole("alert")).toHaveTextContent("The title can't be empty.");
  });
});

describe("failures", () => {
  it("no permission: the error shows and editing hides for that server", async () => {
    const album = released();
    usePlexEdit.getState().open("details", { id: album.id, kind: "album", title: album.title, thumb: album.thumb });
    render(<Hosts />);
    const dlg = await screen.findByRole("dialog", { name: "Edit details" });
    const user = userEvent.setup();
    await user.type(within(dlg).getByLabelText("Record label"), " X");
    mock.plexEdit.failNext("permission");
    mock.plexEdit.shared = true;
    await user.click(within(dlg).getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(toasts().map((t) => t.message)).toContain("Couldn't save the details: Only the server's owner and admins can edit this."),
    );
    await waitFor(() => expect(usePlexEdit.getState().access?.connected).toBe(false));
  });

  it("server offline: the error says so", async () => {
    mock.plexEdit.failNext("offline");
    const user = userEvent.setup();
    const sub = await openEditMenu(user, "album", released());
    await user.click(within(sub).getByRole("menuitem", { name: "Refresh details" }));
    await waitFor(() => expect(toasts().map((t) => t.message)).toContain("Couldn't refresh: Your server is offline."));
  });
});

describe("merge", () => {
  it("finds the other artist, keeps the chosen name, and moves its albums", async () => {
    const [a, b] = mock.fx.artists;
    usePlexEdit.getState().open("merge", { id: a.id, kind: "artist", title: a.title, thumb: a.thumb });
    render(<Hosts />);
    const dlg = await screen.findByRole("dialog", { name: `Merge ${a.title}` });
    const user = userEvent.setup();
    await user.type(within(dlg).getByLabelText("Find an artist"), b.title.slice(0, 4));
    await user.click(await within(dlg).findByRole("option", { name: b.title }));
    const keepNames = within(dlg).getAllByLabelText("Keep this name");
    expect(keepNames[0]).toBeChecked();
    await user.click(within(dlg).getByRole("button", { name: "Merge" }));
    await waitFor(() => expect(mock.fx.artists.some((x) => x.id === b.id)).toBe(false));
    expect(mock.fx.albums.some((x) => x.artistId === b.id)).toBe(false);
    const t = toasts().find((x) => x.message === `Merged another artist into ${a.title}`)!;
    act(() => t.action!.run());
    await waitFor(() => expect(mock.fx.artists.some((x) => x.id === b.id)).toBe(true));
  });
});

describe("settings", () => {
  it("has its three rows, found by search", async () => {
    render(<PlexEditSettings />);
    expect(screen.getByText("Show editing options")).toBeInTheDocument();
    expect(screen.getByText("Lock fields I edit")).toBeInTheDocument();
    expect(screen.getByText("Fix match on unreleased albums")).toBeInTheDocument();
    expect(plexEditMatches("lock")).toBe(true);
    expect(plexEditMatches("crossfade")).toBe(false);
    const user = userEvent.setup();
    await user.click(screen.getByRole("switch", { name: "Lock fields I edit" }));
    await waitFor(async () => expect((await backend.settingsGet()).plexEditLock).toBe(false));
  });
});
