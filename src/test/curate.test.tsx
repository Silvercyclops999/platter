/**
 * Smart playlists (the editor, its live count, saving to Plex or keeping a
 * Platter one, Plex refusing the rules, prompted playlists behind the AI
 * provider), hiding songs and artists, bookmarks, and the scrobbling
 * settings (sign-in and a refused session).
 */
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { ContextMenuHost } from "../components/ContextMenu";
import { SmartEditorHost, completeRules, previewLine } from "../components/SmartPlaylistEditor";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS } from "../lib/types";
import { BookmarksPage } from "../pages/BookmarksPage";
import { MixPage } from "../pages/MixPage";
import { SettingsPage } from "../pages/SettingsPage";
import { SmartPlaylistPage, rulesLine } from "../pages/SmartPlaylistPage";
import { Sidebar } from "../shell/Sidebar";
import { useApp } from "../stores/app";
import { bookmarkNowPlaying, newSmartPlaylist, resetCurate, startCurate, useCurate } from "../stores/curate";
import { useLibraryView } from "../stores/library";
import { usePlayer } from "../stores/player";
import { useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";
import { useUi } from "../stores/ui";

const mock = backend as MockBackend;
const toasts = () => useToasts.getState().toasts.map((t) => t.message);
const route = () => useRouter.getState().entries[useRouter.getState().index].route;

beforeEach(async () => {
  mock.hidden.reset();
  mock.bookmarks.reset(mock.fx);
  mock.smart.reset();
  mock.scrobble.reset();
  await mock.settingsSet({ ...DEFAULT_SETTINGS });
  useSettings.setState({ settings: null });
  await loadSettings();
  useApp.setState({ auth: { state: "signedIn", username: "listener", needsServerPick: false } });
  useToasts.setState({ toasts: [] });
  useUi.setState({ menu: null });
  useRouter.getState().reset({ name: "home" });
  useLibraryView.setState({ filter: "all", sub: null, downloaded: null });
  resetCurate();
  await startCurate();
});

const genre = () => mock.fx.albums[0].genres[0];

async function openEditor(prompt = false) {
  render(
    <>
      <SmartEditorHost />
      <ContextMenuHost />
    </>,
  );
  act(() => newSmartPlaylist({ prompt }));
  return await screen.findByRole("dialog");
}

describe("smart playlist editor", () => {
  it("counts the songs as the rules change, and saves rules Plex can hold to Plex", async () => {
    const user = userEvent.setup();
    const dialog = await openEditor();
    // An empty value is still being typed: everything counts.
    await waitFor(() => expect(screen.getByTestId("smart-count")).toHaveTextContent(`${mock.fx.tracks.length.toLocaleString()} songs`));
    await user.type(within(dialog).getByRole("textbox", { name: "Rule 1 value" }), genre());
    const want = mock.smart.preview({ matchAll: true, rules: [{ field: "genre", op: "is", value: genre() }], sort: "none", update: "live" }).count;
    await waitFor(() => expect(screen.getByTestId("smart-count")).toHaveTextContent(`${want} song`));
    expect(within(dialog).getByText("Saved in Plex, so your other Plex apps see it too.")).toBeInTheDocument();
    await user.type(within(dialog).getByRole("textbox", { name: "Name" }), "Genre night");
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(useCurate.getState().editor).toBeNull());
    const made = mock.fx.playlists.find((p) => p.title === "Genre night")!;
    expect(made.smart).toBe(true);
    expect(route()).toEqual({ name: "playlist", id: made.id });
    expect(toasts()).toContain('Made "Genre night"');
  });

  it("keeps rules Plex can't express as a Platter smart playlist and says why", async () => {
    const user = userEvent.setup();
    const dialog = await openEditor();
    await user.selectOptions(within(dialog).getByRole("combobox", { name: "Rule 1 field" }), "duration");
    await user.type(within(dialog).getByRole("textbox", { name: "Rule 1 value" }), "3");
    await user.selectOptions(within(dialog).getByRole("combobox", { name: "Rule 1 comparison" }), "gt");
    await waitFor(() => expect(within(dialog).getByText(/This will be a Platter smart playlist, filled on this computer: Length/)).toBeInTheDocument());
    await user.selectOptions(within(dialog).getByRole("combobox", { name: "Update" }), "weekly");
    await user.type(within(dialog).getByRole("textbox", { name: "Name" }), "Long ones");
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(useCurate.getState().smart).toHaveLength(1));
    const s = useCurate.getState().smart[0];
    expect(s.id.startsWith("smart:")).toBe(true);
    expect(s.update).toBe("weekly");
    expect(toasts().some((t) => t.startsWith("Saved as a Platter smart playlist on this computer"))).toBe(true);
  });

  it("app state: Plex refuses the rules, so it's saved as a Platter smart playlist, explained", async () => {
    const user = userEvent.setup();
    mock.smart.refusePlex = true;
    const dialog = await openEditor();
    await user.type(within(dialog).getByRole("textbox", { name: "Rule 1 value" }), genre());
    await user.type(within(dialog).getByRole("textbox", { name: "Name" }), "Refused");
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(toasts()).toContain("Plex didn't accept these rules, so this is a Platter smart playlist on this computer."));
    expect(useCurate.getState().smart.map((s) => s.title)).toEqual(["Refused"]);
  });

  it("says why it can't save while the server is offline", async () => {
    const user = userEvent.setup();
    mock.smart.offline = true;
    const dialog = await openEditor();
    await user.type(within(dialog).getByRole("textbox", { name: "Name" }), "Later");
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    expect(await within(dialog).findByRole("alert", {}, { timeout: 4000 })).toHaveTextContent(/server is offline/);
    expect(useCurate.getState().editor).not.toBeNull();
  });

  it("keeps the save error when the song count answers late", async () => {
    const user = userEvent.setup();
    mock.smart.offline = true;
    const dialog = await openEditor();
    await user.type(within(dialog).getByRole("textbox", { name: "Name" }), "Later");
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(/server is offline/);
    // A rule change makes the count ask again; its answer mustn't clear the error.
    await user.type(within(dialog).getByRole("textbox", { name: "Rule 1 value" }), genre());
    await new Promise((r) => setTimeout(r, 400));
    expect(within(dialog).getByRole("alert")).toHaveTextContent(/server is offline/);
  });

  it("adds and removes rules, matches any, and limits", async () => {
    const user = userEvent.setup();
    const dialog = await openEditor();
    await user.click(within(dialog).getByRole("button", { name: "Add rule" }));
    expect(within(dialog).getAllByRole("listitem")).toHaveLength(2);
    await user.click(within(dialog).getByRole("button", { name: "Remove rule 1" }));
    expect(within(dialog).getAllByRole("listitem")).toHaveLength(1);
    await user.selectOptions(within(dialog).getByRole("combobox", { name: "Songs that match" }), "any");
    await user.selectOptions(within(dialog).getByRole("combobox", { name: "Limit" }), "songs");
    const n = within(dialog).getByRole("textbox", { name: "Number of songs" });
    await user.clear(n);
    await user.type(n, "5");
    await waitFor(() => expect(screen.getByTestId("smart-count")).toHaveTextContent(/^5 songs/));
  });

  it("names the rules on a Platter playlist's page", () => {
    expect(rulesLine({ matchAll: false, rules: [{ field: "genre", op: "is", value: "Jazz" }, { field: "liked", op: "is", value: "true" }], sort: "none", update: "live" })).toBe(
      "Genre is Jazz, or Liked is yes",
    );
    expect(completeRules({ matchAll: true, rules: [{ field: "genre", op: "is", value: " " }], sort: "none", update: "live" }).rules).toEqual([]);
    expect(previewLine({ count: 0, durationMs: 0, tracks: [], plexGaps: [] })).toBe("No songs match yet");
    expect(previewLine({ count: 12, durationMs: 75 * 60_000, tracks: [], plexGaps: [] })).toBe("12 songs, 1 hr 15 min");
  });

  it("shows a Platter smart playlist with its mark and plays its songs", async () => {
    const saved = await backend.smartSave({ title: "Mine", rules: { matchAll: true, rules: [{ field: "playCount", op: "gt", value: "0" }], sort: "mostPlayed", update: "live" }, platter: true });
    render(<SmartPlaylistPage id={saved.id} />);
    expect(await screen.findByRole("heading", { name: "Mine" })).toBeInTheDocument();
    expect(screen.getByText("Platter")).toBeInTheDocument();
    expect(screen.getByText(/Plays over 0/)).toBeInTheDocument();
  });
});

describe("prompted playlists", () => {
  it("stay hidden until an AI provider and key are set, then fill the rules and picks", async () => {
    const user = userEvent.setup();
    const { unmount } = render(
      <>
        <Sidebar collapsed={false} />
        <ContextMenuHost />
      </>,
    );
    await user.click(await screen.findByRole("button", { name: "Create playlist or folder" }));
    let menu = await screen.findByRole("menu");
    expect(within(menu).getByRole("menuitem", { name: "Smart playlist" })).toBeInTheDocument();
    expect(within(menu).queryByRole("menuitem", { name: "Prompted playlist" })).toBeNull();
    act(() => useUi.getState().closeMenu());

    await mock.settingsSet({ aiProvider: "anthropic" });
    await backend.aiKeySet("test-key");
    useCurate.setState({ ai: await backend.aiStatus() });
    await user.click(screen.getByRole("button", { name: "Create playlist or folder" }));
    menu = await screen.findByRole("menu");
    await user.click(within(menu).getByRole("menuitem", { name: "Prompted playlist" }));
    unmount();

    useCurate.setState({ editor: null });
    const dialog = await openEditor(true);
    await user.type(within(dialog).getByRole("textbox", { name: "Describe your playlist" }), `${genre()} for a long drive`);
    await user.click(within(dialog).getByRole("button", { name: "Make rules" }));
    await waitFor(() => expect(within(dialog).getByRole("textbox", { name: "Rule 1 value" })).toHaveValue(genre()));
    expect(within(dialog).getByRole("region", { name: "Picked songs" })).toBeInTheDocument();
    expect(within(dialog).getByText(/Picked songs stay first/)).toBeInTheDocument();
    expect(within(dialog).getByText(/Unreleased music is never sent/)).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(useCurate.getState().smart[0]?.prompted).toBe(true));
  });
});

describe("hide", () => {
  it("Hide song on a mix takes it off the page and out of the mix", async () => {
    const user = userEvent.setup();
    const home = await backend.mixesHome();
    const mix = home.madeForYou[0];
    render(
      <>
        <MixPage id={mix.id} />
        <ContextMenuHost />
      </>,
    );
    const page = await backend.mixPage(mix.id);
    const t = page.tracks.find((x) => page.tracks.filter((y) => y.title.includes(x.title) || x.title.includes(y.title)).length === 1)!;
    const esc = t.title.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    fireEvent.contextMenu(await screen.findByRole("row", { name: new RegExp(esc) }));
    const menu = await screen.findByRole("menu");
    await user.click(within(menu).getByRole("menuitem", { name: "Hide song" }));
    await waitFor(() => expect(useCurate.getState().hiddenTracks.has(t.id)).toBe(true));
    await waitFor(() => expect(screen.queryByRole("row", { name: new RegExp(esc) })).toBeNull());
    // The core leaves it out of the mix too.
    expect((await backend.mixPage(mix.id)).tracks.some((x) => x.id === t.id)).toBe(false);
    expect(toasts().some((m) => m.startsWith("Hidden."))).toBe(true);
  });

  it("Don't play this artist leaves shuffle play, and Settings lists it with Unhide", async () => {
    const user = userEvent.setup();
    const a = mock.fx.artists[0];
    await backend.hiddenSet({ kind: "artist", id: a.id, title: a.title }, true);
    const album = mock.fx.albums.find((x) => x.artistId === a.id)!;
    const ids = [...mock.fx.tracks.filter((t) => t.albumId === album.id).map((t) => t.id), ...mock.fx.tracks.filter((t) => t.artistId !== a.id).slice(0, 3).map((t) => t.id)];
    await backend.playContext({ context: { kind: "tracks", trackIds: ids }, shuffle: true });
    const st = await backend.playerState();
    const all = [st.track!, ...st.queue.nextUp.map((i) => i.track)];
    expect(all.every((t) => t.artistId !== a.id)).toBe(true);

    render(<SettingsPage />);
    const list = await screen.findByRole("list", { name: "Hidden songs and artists" });
    expect(within(list).getByText(a.title)).toBeInTheDocument();
    await user.click(within(list).getByRole("button", { name: `Unhide ${a.title}` }));
    await waitFor(() => expect(useCurate.getState().hidden).toHaveLength(0));
  });
});

describe("bookmarks", () => {
  it("the shortcut bookmarks the playing song at its time; the page plays it from there", async () => {
    const user = userEvent.setup();
    const t = mock.fx.tracks[5];
    await backend.playContext({ context: { kind: "tracks", trackIds: [t.id] } });
    usePlayer.getState().set(await backend.playerState());
    act(() => usePlayer.getState().setPosition(95_000, t.durationMs));
    await act(async () => bookmarkNowPlaying());
    await waitFor(() => expect(useCurate.getState().bookmarks.some((b) => b.target === t.id && b.positionMs === 95_000)).toBe(true));
    expect(toasts()).toContain("Bookmarked at 1:35");

    render(<BookmarksPage />);
    await user.click(await screen.findByRole("button", { name: `Play ${t.title} from 1:35` }));
    // The mock keeps playing, so allow the few seconds it may have moved on.
    await waitFor(async () => {
      const at = (await backend.playerState()).positionMs;
      expect(at).toBeGreaterThanOrEqual(95_000);
      expect(at).toBeLessThan(99_000);
    });
    // A page bookmark opens its page.
    const page = useCurate.getState().bookmarks.find((b) => b.kind === "page")!;
    await user.click(screen.getByRole("button", { name: `Open ${page.title}` }));
    expect(route()).toEqual({ name: "album", id: page.target });
    await user.click(screen.getByRole("button", { name: `Remove the bookmark for ${page.title}` }));
    await waitFor(() => expect(useCurate.getState().bookmarks.some((b) => b.id === page.id)).toBe(false));
  });

  it("Bookmarks has a row in the library while its setting is on", async () => {
    const { unmount } = render(<Sidebar collapsed={false} />);
    expect(await screen.findByText("Bookmarks")).toBeInTheDocument();
    unmount();
    act(() => useSettings.setState({ settings: { ...useSettings.getState().settings!, bookmarksInLibrary: false } }));
    render(<Sidebar collapsed={false} />);
    await screen.findByText("Liked Songs");
    expect(screen.queryByText("Bookmarks")).toBeNull();
  });
});

describe("scrobbling settings", () => {
  it("signs in to Last.fm in the browser, then scrobbling can be turned on", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    const secret = await screen.findByLabelText("Last.fm shared secret");
    await user.type(secret, "s3");
    await user.click(screen.getByRole("button", { name: "Save secret" }));
    await user.click(await screen.findByRole("button", { name: "Sign in on Last.fm" }));
    expect(await screen.findByText(/Allow Platter on the Last.fm page/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Finish sign-in" }));
    expect((await screen.findAllByText(/Signed in as listener/)).length).toBeGreaterThan(0);
    const row = document.querySelector('[data-setting="scrobbleLastfm"]') as HTMLElement;
    await user.click(within(row).getByRole("switch"));
    await waitFor(() => expect(useSettings.getState().settings?.scrobbleLastfm).toBe(true));
  });

  it("app state: the service refuses the session, so Settings asks to sign in again", async () => {
    await backend.lastfmSecretSet("s");
    await backend.lastfmSignInStart();
    await backend.lastfmSignInFinish();
    render(<SettingsPage />);
    await screen.findAllByText(/Signed in as listener/);
    act(() => mock.scrobble.reject("lastfm"));
    expect((await screen.findAllByText(/Last.fm signed Platter out/)).length).toBeGreaterThan(0);
    expect(screen.getByRole("button", { name: "Sign in again" })).toBeInTheDocument();
  });

  it("ListenBrainz needs the token from Artist info", async () => {
    render(<SettingsPage />);
    expect((await screen.findAllByText("Add your ListenBrainz user token under Artist info first.")).length).toBeGreaterThan(0);
  });
});
