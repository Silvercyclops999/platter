/**
 * The tag editor: menu entries, view-only details without a music folder,
 * editing with "(several)" left alone, saving with Undo, a file changed
 * since the editor opened, the 50-file question, the tag check with Fix and
 * "This is fine", fixes with previews (release status ticked one by one,
 * ISRCs from a picked release, the lookup service down), marking explicit,
 * and the settings.
 */
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { ContextMenuHost } from "../components/ContextMenu";
import { TagEditDialogs } from "../components/TagEditDialogs";
import { Toaster } from "../components/Toast";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { MANY_FILES, asksFirst, buildRequest, fixRequest, joinReleaseType, shared, splitReleaseType } from "../lib/tagEdit";
import { DEFAULT_SETTINGS, type Album, type TagEditFile } from "../lib/types";
import { AlbumCard } from "../pages/cards";
import { MusicFolderControl, MusicFolderNote, PlexLibrarySettings } from "../pages/TagEditorSettings";
import { connectApp } from "../stores/app";
import { useSettings } from "../stores/settings";
import { useTagEdit } from "../stores/tagEdit";
import { useToasts } from "../stores/toasts";
import { useUi } from "../stores/ui";

const mock = backend as MockBackend;
const toasts = () => useToasts.getState().toasts;

let disconnect: () => void;
beforeAll(() => {
  disconnect = connectApp(mock);
});
afterAll(() => disconnect());

async function withFolder(on: boolean) {
  const patch = on ? { musicFolder: "/Volumes/Music", musicFolderServer: "/data/music" } : { musicFolder: "", musicFolderServer: "" };
  await mock.settingsSet({ ...DEFAULT_SETTINGS, ...patch });
  useSettings.setState({ settings: { ...DEFAULT_SETTINGS, ...patch } });
}

beforeEach(async () => {
  useToasts.setState({ toasts: [] });
  useUi.setState({ menu: null });
  useTagEdit.setState({ editor: null, check: null, fix: null, explicit: null });
  mock.tagEdit.failNext = null;
  mock.tagEdit.lookupDown = false;
  mock.tagEdit.folderGone = false;
  await withFolder(true);
});

/** A released album with a few songs. */
const album = (): Album => mock.fx.albums.find((a) => a.format === "album" && (mock.fx.tracksByAlbum.get(a.id)?.length ?? 0) >= 3)!;
const songs = (a: Album) => mock.fx.tracksByAlbum.get(a.id)!;

function Hosts() {
  return (
    <>
      <ContextMenuHost />
      <TagEditDialogs />
      <Toaster />
    </>
  );
}

function file(values: TagEditFile["values"], id = "t1"): TagEditFile {
  return { trackId: id, albumId: "al1", serverPath: null, localPath: null, fileName: `${id}.flac`, format: "flac", id3Version: null, values, keys: [], stamp: `${id}:1`, error: null, durationMs: 1000 };
}

describe("editor logic", () => {
  it("album values differ: (several), and left alone unless edited", () => {
    const files = [file({ album: "Demo Album", genre: "Pop" }, "t1"), file({ album: "Demo Album (Deluxe)", genre: "Pop" }, "t2")];
    expect(shared(files, "album")).toBeNull();
    expect(shared(files, "genre")).toBe("Pop");
    const r = buildRequest("x", files, { genre: "Hip Hop" }, { t2: { title: " New title " } });
    expect(r.files).toEqual([
      { trackId: "t1", stamp: "t1:1", edits: { genre: "Hip Hop" } },
      { trackId: "t2", stamp: "t2:1", edits: { genre: "Hip Hop", title: "New title" } },
    ]);
    // A value a file already has isn't sent; unreadable files are left out.
    const r2 = buildRequest("x", [file({ genre: "Pop" }), { ...file({}, "t9"), stamp: null }], { genre: "Pop" }, {});
    expect(r2.files).toEqual([]);
  });

  it("release type in two parts", () => {
    expect(splitReleaseType("album;demo")).toEqual({ primary: "album", secondary: "demo" });
    expect(splitReleaseType("demo")).toEqual({ primary: "", secondary: "demo" });
    expect(joinReleaseType("ep", "")).toBe("ep");
    expect(joinReleaseType("", "live")).toBe("album;live");
    expect(joinReleaseType("", "")).toBe("");
  });

  it("asks before more than 50 files unless turned off", () => {
    expect(asksFirst(MANY_FILES, true)).toBe(false);
    expect(asksFirst(MANY_FILES + 1, true)).toBe(true);
    expect(asksFirst(MANY_FILES + 1, false)).toBe(false);
    const r = fixRequest("Filled disc tags", [
      { trackId: "t1", fileName: "a", stamp: "s1", field: "discNumber", before: null, after: "1", note: null },
      { trackId: "t1", fileName: "a", stamp: "s1", field: "discTotal", before: null, after: "1", note: null },
    ]);
    expect(r.files).toEqual([{ trackId: "t1", stamp: "s1", edits: { discNumber: "1", discTotal: "1" } }]);
  });
});

describe("the editor", () => {
  it("album menus offer Edit tags and Check tags", async () => {
    const user = userEvent.setup();
    const a = album();
    render(
      <>
        <AlbumCard album={a} />
        <Hosts />
      </>,
    );
    fireEvent.contextMenu(screen.getByText(a.title), { clientX: 10, clientY: 10 });
    expect(await screen.findByRole("menuitem", { name: "Check tags…" })).toBeInTheDocument();
    expect(screen.getByRole("menuitem", { name: "Mark as explicit…" })).toBeInTheDocument();
    await user.click(screen.getByRole("menuitem", { name: "Edit tags…" }));
    const d = await screen.findByRole("dialog", { name: "Edit tags" });
    expect(await within(d).findByLabelText("Album artist")).toHaveValue(a.artistTitle);
  });

  it("without a music folder it's view only and Save sets it up", async () => {
    await withFolder(false);
    const a = album();
    render(<Hosts />);
    act(() => useTagEdit.getState().openEditor({ albumId: a.id, trackIds: [] }));
    const d = await screen.findByRole("dialog", { name: "Edit tags" });
    expect(await within(d).findByText(/Set up the music folder/)).toBeInTheDocument();
    expect(within(d).getByLabelText("Album")).toBeDisabled();
    expect(within(d).getByRole("button", { name: "Set up music folder" })).toBeEnabled();
  });

  it("saves only what changed, then Undo puts it back", async () => {
    const user = userEvent.setup();
    const a = album();
    render(<Hosts />);
    act(() => useTagEdit.getState().openEditor({ albumId: a.id, trackIds: [] }));
    const d = await screen.findByRole("dialog", { name: "Edit tags" });
    const genre = await within(d).findByLabelText("Genre");
    await user.type(genre, "Demo Genre");
    await user.click(within(d).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Edit tags" })).not.toBeInTheDocument());
    const t = toasts().find((x) => x.message.startsWith("Saved"))!;
    expect(t.message).toBe(`Saved ${songs(a).length} files`);
    const opened = await backend.tagEditOpen(a.id, []);
    expect(opened.files.every((f) => f.values.genre === "Demo Genre")).toBe(true);
    await act(async () => t.action!.run());
    await waitFor(() => expect(toasts().some((x) => x.message.startsWith("Undid the edit"))).toBe(true));
    expect((await backend.tagEditOpen(a.id, [])).files.every((f) => !f.values.genre)).toBe(true);
  });

  it("a file changed since the editor opened stops the save, nothing written", async () => {
    const user = userEvent.setup();
    const a = album();
    render(<Hosts />);
    act(() => useTagEdit.getState().openEditor({ albumId: a.id, trackIds: [] }));
    const d = await screen.findByRole("dialog", { name: "Edit tags" });
    await user.type(await within(d).findByLabelText("Era"), "Demo Era");
    mock.tagEdit.changeOnDisk(songs(a)[1].id);
    await user.click(within(d).getByRole("button", { name: "Save" }));
    expect(await within(d).findByText(/changed since the editor opened/)).toBeInTheDocument();
    expect((await backend.tagEditOpen(a.id, [])).files.some((f) => f.values.era)).toBe(false);
  });

  it("a read-only share or a locked file says so", async () => {
    const user = userEvent.setup();
    const a = album();
    render(<Hosts />);
    for (const [fail, text] of [
      ["readOnly", /read-only for this account/],
      ["locked", /open in another program/],
      ["busy", /Another program is changing this folder/],
    ] as const) {
      act(() => useTagEdit.getState().openEditor({ albumId: a.id, trackIds: [] }));
      const d = await screen.findByRole("dialog", { name: "Edit tags" });
      await user.clear(await within(d).findByLabelText("Edition"));
      await user.type(within(d).getByLabelText("Edition"), `Deluxe ${fail}`);
      mock.tagEdit.failNext = fail;
      await user.click(within(d).getByRole("button", { name: "Save" }));
      expect(await within(d).findByText(text)).toBeInTheDocument();
      act(() => useTagEdit.getState().close("editor"));
    }
  });

  it("a share that's gone leaves the files unreadable", async () => {
    mock.tagEdit.folderGone = true;
    render(<Hosts />);
    act(() => useTagEdit.getState().openEditor({ albumId: album().id, trackIds: [] }));
    const d = await screen.findByRole("dialog", { name: "Edit tags" });
    expect(await within(d).findByText(/Check that the share is connected/)).toBeInTheDocument();
  });

  it("one song from its row", async () => {
    const t = songs(album())[0];
    render(<Hosts />);
    act(() => useTagEdit.getState().openEditor({ albumId: null, trackIds: [t.id] }));
    const d = await screen.findByRole("dialog", { name: "Edit tags" });
    expect(await within(d).findByLabelText(`Title, ${t.title}`)).toHaveValue(t.title);
    expect(within(d).queryByRole("button", { name: "Fill disc tags" })).not.toBeInTheDocument();
  });
});

describe("the tag check and fixes", () => {
  it("lists what's wrong with why, Fix opens the editor, This is fine hides it", async () => {
    const user = userEvent.setup();
    const a = album();
    // A song without an album artist.
    await backend.tagEditSave({ label: "x", files: [{ trackId: songs(a)[0].id, stamp: (await backend.tagEditOpen(a.id, [])).files[0].stamp!, edits: { albumArtist: "" } }] });
    render(<Hosts />);
    act(() => useTagEdit.getState().openCheck({ kind: "album", id: a.id }, a.title));
    const d = await screen.findByRole("dialog", { name: "Check tags" });
    expect(await within(d).findByText("1 song without an album artist")).toBeInTheDocument();
    expect(within(d).getByText(/treats the album as Various Artists/)).toBeInTheDocument();
    // A missing ISRC is information only.
    expect(within(d).getByText(/released without an ISRC/).closest("li")).toHaveClass("te-finding--info");
    await user.click(within(within(d).getByText("1 song without an album artist").closest("li")!).getByRole("button", { name: "This is fine" }));
    expect(within(d).queryByText("1 song without an album artist")).not.toBeInTheDocument();
    await user.click(within(within(d).getByText(/released without an ISRC/).closest("li")!).getByRole("button", { name: "Fix" }));
    expect(await screen.findByRole("dialog", { name: "Edit tags" })).toBeInTheDocument();
  });

  it("without a music folder there's no Fix", async () => {
    await withFolder(false);
    render(<Hosts />);
    act(() => useTagEdit.getState().openCheck({ kind: "album", id: album().id }, "x"));
    const d = await screen.findByRole("dialog", { name: "Check tags" });
    expect(await within(d).findByText(/Read from what Plex has/)).toBeInTheDocument();
    expect(within(d).queryByRole("button", { name: "Fix" })).not.toBeInTheDocument();
  });

  it("fill disc tags previews every change, applies, and can be undone", async () => {
    const user = userEvent.setup();
    const a = album();
    render(<Hosts />);
    act(() => useTagEdit.getState().openFix({ kind: "discTags", albumId: a.id, title: a.title }));
    const d = await screen.findByRole("dialog", { name: "Fill disc tags" });
    expect(await within(d).findByText(`${songs(a).length * 2} changes to make.`)).toBeInTheDocument();
    await user.click(within(d).getByRole("button", { name: `Change ${songs(a).length} files` }));
    await waitFor(() => expect(toasts().some((t) => t.message === `Saved ${songs(a).length} files`)).toBe(true));
    expect((await backend.tagEditOpen(a.id, [])).files.every((f) => f.values.discNumber === "1" && f.values.discTotal === "1")).toBe(true);
    expect((await backend.tagEditHistory())[0].label).toBe(`Filled disc tags on ${a.title}`);
  });

  it("release status suggestions start unticked", async () => {
    const user = userEvent.setup();
    const a = album();
    const opened = await backend.tagEditOpen(a.id, []);
    await backend.tagEditSave({ label: "x", files: [{ trackId: opened.files[0].trackId, stamp: opened.files[0].stamp!, edits: { archiveVersion: "" } }] });
    render(<Hosts />);
    act(() => useTagEdit.getState().openFix({ kind: "releaseStatus", albumId: a.id, title: a.title }));
    const d = await screen.findByRole("dialog", { name: "Fill release status" });
    const box = await within(d).findByRole("checkbox");
    expect(box).not.toBeChecked();
    expect(within(d).getByRole("button", { name: "Change 0 files" })).toBeDisabled();
    await user.click(box);
    await user.click(within(d).getByRole("button", { name: "Change 1 file" }));
    await waitFor(() => expect(toasts().some((t) => t.message === "Saved 1 file")).toBe(true));
  });

  it("ISRCs come from the one release picked; re-edits are kept as they are", async () => {
    const user = userEvent.setup();
    const a = album();
    render(<Hosts />);
    act(() => useTagEdit.getState().openFix({ kind: "isrc", albumId: a.id, title: a.title }));
    const d = await screen.findByRole("dialog", { name: "Look up ISRCs" });
    const radios = await within(d).findAllByRole("radio");
    expect(within(d).getByRole("button", { name: "Use this release" })).toBeDisabled();
    await user.click(radios[0]);
    await user.click(within(d).getByRole("button", { name: "Use this release" }));
    expect(await within(d).findByText(/Kept as they are/)).toBeInTheDocument();
    expect(within(d).getAllByText(/likely another edit/).length).toBeGreaterThan(0);
  });

  it("the lookup service down: nothing changes and it says so", async () => {
    mock.tagEdit.lookupDown = true;
    const a = album();
    render(<Hosts />);
    act(() => useTagEdit.getState().openFix({ kind: "isrc", albumId: a.id, title: a.title }));
    const d = await screen.findByRole("dialog", { name: "Look up ISRCs" });
    expect(await within(d).findByText(/couldn't be reached/)).toBeInTheDocument();
  });

  it("mark an album explicit", async () => {
    const user = userEvent.setup();
    const a = album();
    render(<Hosts />);
    act(() => useTagEdit.getState().openExplicit({ albumIds: [a.id], explicit: true, title: a.title }));
    const d = await screen.findByRole("dialog", { name: "Mark as explicit" });
    await user.click(await within(d).findByRole("button", { name: `Change ${songs(a).length} files` }));
    await waitFor(() => expect(toasts().some((t) => t.message === `Saved ${songs(a).length} files`)).toBe(true));
    expect((await backend.tagEditHistory())[0].label).toBe(`Marked ${a.title} as explicit`);
  });
});

describe("settings", () => {
  it("choosing the music folder checks 5 files", async () => {
    await withFolder(false);
    const user = userEvent.setup();
    render(
      <>
        <MusicFolderNote />
        <MusicFolderControl />
        <Toaster />
      </>,
    );
    expect(screen.getByText(/Not set/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Choose…" }));
    await waitFor(() => expect(toasts().some((t) => t.message === "Connected, 5 of 5 files found")).toBe(true));
    act(() => useSettings.setState({ settings: { ...DEFAULT_SETTINGS, musicFolder: "/Volumes/Music", musicFolderServer: "/data/music" } }));
    expect(await screen.findByText("/Volumes/Music: Connected, 5 of 5 files found")).toBeInTheDocument();
  });

  it("Plex library settings are explained, read only", async () => {
    render(<PlexLibrarySettings />);
    expect(await screen.findByText("Local metadata preferred")).toBeInTheDocument();
    expect(screen.getByText(/turn on Prefer local metadata/)).toBeInTheDocument();
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });
});
