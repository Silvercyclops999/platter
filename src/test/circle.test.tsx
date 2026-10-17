/**
 * Shared playlists and Blends: the invite button and dialog (people on your
 * server or Home, then a role; the invite link), the byline and Playlist
 * permissions (private or public, roles, removing someone, your own role),
 * the Added by column and its sort, a playlist someone shared with you
 * (Not in your library rows, removing only your own songs, leaving by
 * case), edits that wait while the owner is offline, the Blend page (Added
 * by, hiding a song for good, Leave Blend), Create a Blend (the 10-person
 * cap), the Create menu, invites that came straight to you, and the failure
 * states: relay down, owner unreachable, invite expired.
 */
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { CircleDialogs, leaveMessage } from "../components/CircleDialogs";
import { ContextMenuHost } from "../components/ContextMenu";
import { PlaylistDialogs } from "../components/PlaylistDialogs";
import { collaboratorPlaylists, editablePlaylists } from "../components/PlaylistPicker";
import { Toaster } from "../components/Toast";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { MOCK_BLEND_ID, MOCK_COLLAB_INVITE, MOCK_OWNED_SHARED, MOCK_SHARED_ID } from "../lib/mock/circle";
import { DEFAULT_SETTINGS, type SharedSummary } from "../lib/types";
import { BlendCreatePage } from "../pages/BlendCreatePage";
import { BlendPage } from "../pages/BlendPage";
import { PlaylistPage } from "../pages/PlaylistPage";
import { SharedPlaylistPage } from "../pages/SharedPlaylistPage";
import { Sidebar } from "../shell/Sidebar";
import { connectApp, useApp } from "../stores/app";
import { bylineText, joinCircle, resetCircle, startCircle, useCircle } from "../stores/circle";
import { useLibraryView } from "../stores/library";
import { useRouter } from "../stores/router";
import { useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";
import { useUi } from "../stores/ui";

const mock = backend as MockBackend;
const toasts = () => useToasts.getState().toasts.map((t) => t.message);
const route = () => useRouter.getState().entries[useRouter.getState().index].route;
let clip: string[] = [];

let disconnect: () => void;
beforeAll(() => {
  disconnect = connectApp(mock);
});
afterAll(() => disconnect());

beforeEach(async () => {
  clip = [];
  Object.defineProperty(navigator, "clipboard", { value: { writeText: vi.fn(async (t: string) => void clip.push(t)) }, configurable: true });
  await mock.settingsSet({ ...DEFAULT_SETTINGS });
  useSettings.setState({ settings: { ...DEFAULT_SETTINGS } });
  useApp.setState({ auth: { state: "signedIn", username: "listener", needsServerPick: false } });
  useToasts.setState({ toasts: [] });
  useUi.setState({ menu: null });
  useRouter.getState().reset({ name: "home" });
  useLibraryView.setState({ filter: "all", sub: null });
  mock.circle.reset();
  resetCircle();
  await startCircle();
  const lists = await backend.playlists();
  act(() => useApp.getState().setPlaylists(lists));
});

function Hosts() {
  return (
    <>
      <ContextMenuHost />
      <PlaylistDialogs />
      <CircleDialogs />
      <Toaster />
    </>
  );
}

const unsharedId = "500";

describe("inviting to your playlist", () => {
  it("has a person-with-a-plus button between download and more, with the same item in the more menu", async () => {
    const user = userEvent.setup();
    render(
      <>
        <PlaylistPage id={unsharedId} />
        <Hosts />
      </>,
    );
    const title = mock.fx.playlists.find((p) => p.id === unsharedId)!.title;
    const invite = await screen.findByRole("button", { name: `Invite collaborators to ${title}` });
    const row = invite.closest(".action-row")!;
    const buttons = Array.from(row.querySelectorAll("button")).map((b) => b.getAttribute("aria-label") ?? "");
    const at = buttons.indexOf(`Invite collaborators to ${title}`);
    expect(buttons[at - 1]).toMatch(/[Dd]ownload/);
    expect(buttons[at + 1]).toBe(`More options for ${title}`);
    await user.click(screen.getByRole("button", { name: `More options for ${title}` }));
    await user.click(await screen.findByRole("menuitem", { name: "Invite collaborators" }));
    expect(await screen.findByRole("dialog", { name: `Invite collaborators to ${title}` })).toBeInTheDocument();
  });

  it("lists Home users without an invite, asks for a role, and copies a one-use link", async () => {
    const user = userEvent.setup();
    // After user-event's own clipboard.
    Object.defineProperty(navigator, "clipboard", { value: { writeText: vi.fn(async (t: string) => void clip.push(t)) }, configurable: true });
    render(
      <>
        <PlaylistPage id={unsharedId} />
        <Hosts />
      </>,
    );
    await user.click(await screen.findByRole("button", { name: /^Invite collaborators to/ }));
    const dialog = await screen.findByRole("dialog");
    const home = await within(dialog).findByRole("region", { name: "Your Plex Home" });
    expect(within(home).getByRole("button", { name: /Sam/ })).toBeInTheDocument();
    expect(within(dialog).getByRole("region", { name: "On your server" })).toBeInTheDocument();
    await user.click(within(home).getByRole("button", { name: /Sam/ }));
    const roles = within(dialog).getByRole("radiogroup", { name: "Role for Sam" });
    expect(within(roles).getByRole("radio", { name: /Listener/ })).toHaveAttribute("aria-checked", "true");
    await user.click(within(roles).getByRole("radio", { name: /Collaborator/ }));
    await user.click(within(dialog).getByRole("button", { name: "Invite Sam" }));
    await waitFor(() => expect(toasts()).toContain("Invited Sam"));
    // Sam opens it: the playlist is now shared with them, as a collaborator.
    await waitFor(() => expect(useCircle.getState().view.playlists.find((p) => p.id === unsharedId)?.people.map((p) => [p.name, p.role])).toContainEqual(["Sam", "collaborator"]));
    await user.click(within(dialog).getByRole("checkbox", { name: "They can add songs" }));
    await user.click(within(dialog).getByRole("button", { name: "Copy invite link" }));
    await waitFor(() => expect(clip[0]).toMatch(/^platter:\/\/collab\/[A-Za-z0-9_-]+$/));
  });
});

describe("a shared playlist you own", () => {
  it("shows the byline with faces, an Added by column and an Added by sort", async () => {
    const user = userEvent.setup();
    render(
      <>
        <PlaylistPage id={MOCK_OWNED_SHARED} />
        <Hosts />
      </>,
    );
    const byline = await screen.findByRole("button", { name: "You and 2 others. Playlist permissions" });
    expect(byline.querySelectorAll(".face")).toHaveLength(3);
    expect(await screen.findByRole("columnheader", { name: "Added by" })).toBeInTheDocument();
    await waitFor(() => expect(document.querySelectorAll(".track-row__by .truncate")).toHaveLength(2));
    expect(document.querySelector(".track-row__by .truncate")!.textContent).toBe("Robin");
    await user.click(screen.getByRole("button", { name: /^Sort songs/ }));
    await user.click(await screen.findByRole("menuitemradio", { name: "Added by" }));
    // The owner's songs first, then Robin's.
    const by = Array.from(document.querySelectorAll(".track-row")).map((r) => r.querySelector(".track-row__by .truncate")?.textContent ?? "");
    expect(by.slice(-2)).toEqual(["Robin", "Robin"]);
  });

  it("leaves Added by off playlists that aren't shared", async () => {
    render(<PlaylistPage id={unsharedId} />);
    await screen.findAllByRole("row");
    expect(screen.queryByRole("columnheader", { name: "Added by" })).toBeNull();
  });

  it("opens Playlist permissions: public or private, who it's shared with, and each person's menu", async () => {
    const user = userEvent.setup();
    render(
      <>
        <PlaylistPage id={MOCK_OWNED_SHARED} />
        <Hosts />
      </>,
    );
    await user.click(await screen.findByRole("button", { name: /Playlist permissions$/ }));
    const dialog = await screen.findByRole("dialog", { name: "Playlist permissions" });
    expect(within(dialog).getByRole("radio", { name: /Private/ })).toHaveAttribute("aria-checked", "true");
    expect(within(dialog).getByText("listener (you)")).toBeInTheDocument();
    expect(within(dialog).getByText("2 songs added")).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Robin: Collaborator" }));
    const menu = await screen.findByRole("menu");
    expect(Array.from(menu.querySelectorAll('[role^="menuitem"]')).map((m) => m.textContent)).toEqual(["Make collaborator", "Make listener", "Remove from playlist"]);
    await user.click(within(menu).getByRole("menuitemradio", { name: "Make listener" }));
    await waitFor(() => expect(toasts()).toContain("Robin can listen only now"));
    await user.click(await within(dialog).findByRole("button", { name: "Jules: Listener" }));
    await user.click(await screen.findByRole("menuitem", { name: "Remove from playlist" }));
    await waitFor(() => expect(toasts()).toContain("Removed Jules from the playlist"));
    await waitFor(() => expect(within(dialog).queryByText("Jules")).toBeNull());
    await user.click(within(dialog).getByRole("radio", { name: /Public/ }));
    await waitFor(() => expect(within(dialog).getByRole("radio", { name: /Public/ })).toHaveAttribute("aria-checked", "true"));
  });

  it("shows a collaborator's song as theirs when it lands", async () => {
    render(
      <>
        <PlaylistPage id={MOCK_OWNED_SHARED} />
        <Hosts />
      </>,
    );
    await screen.findByRole("columnheader", { name: "Added by" });
    const before = document.querySelectorAll(".track-row[data-row-key]").length;
    act(() => mock.circle.memberAdds("Robin", mock.fx.tracks[3].id));
    await waitFor(() => expect(document.querySelectorAll(".track-row[data-row-key]").length).toBe(before + 1));
    const last = Array.from(document.querySelectorAll(".track-row[data-row-key]")).pop()!;
    await waitFor(() => expect(last.querySelector(".track-row__by .truncate")?.textContent).toBe("Robin"));
  });
});

describe("a playlist shared with you", () => {
  const page = () =>
    render(
      <>
        <SharedPlaylistPage id={MOCK_SHARED_ID} />
        <Hosts />
      </>,
    );

  it("plays what you have and marks the rest Not in your library", async () => {
    page();
    expect(await screen.findByRole("heading", { name: "Road Trip" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Morgan and 2 others. Playlist permissions" })).toBeInTheDocument();
    const missing = await screen.findAllByRole("row", { name: /not in your library/ });
    expect(missing.map((r) => r.getAttribute("aria-label"))).toEqual(["Harbour Lights by The Night Ferries, not in your library", "Paper Planes Again by Velvet Static, not in your library"]);
    // Members can't rename it or change its cover.
    expect(screen.queryByRole("button", { name: /Edit details/ })).toBeNull();
    expect(screen.queryByRole("button", { name: /^Invite collaborators/ })).toBeNull();
  });

  it("sends a missing song's album to the Missing list", async () => {
    const user = userEvent.setup();
    page();
    await user.click(await screen.findByRole("button", { name: "More options for Harbour Lights by The Night Ferries" }));
    await user.click(await screen.findByRole("menuitem", { name: "Add to Missing list" }));
    await waitFor(() => expect(toasts()).toContain("Added Coastline to your Missing list"));
    expect((await backend.importMissingAlbums()).some((a) => a.title === "Coastline")).toBe(true);
  });

  it("lets a collaborator remove only the songs they added", async () => {
    const user = userEvent.setup();
    page();
    await screen.findByRole("heading", { name: "Road Trip" });
    const rows = await screen.findAllByRole("row");
    const mine = rows.filter((r) => r.querySelector(".track-row__by .truncate")?.textContent === "listener");
    const others = rows.filter((r) => r.querySelector(".track-row__by .truncate")?.textContent === "Casey" && r.hasAttribute("data-track-id"));
    expect(mine.length).toBeGreaterThan(0);
    await user.pointer({ keys: "[MouseRight]", target: others[0] });
    expect(within(await screen.findByRole("menu")).queryByRole("menuitem", { name: "Remove from this playlist" })).toBeNull();
    act(() => useUi.getState().closeMenu());
    const before = (await backend.collabPage(MOCK_SHARED_ID)).rows.length;
    await user.pointer({ keys: "[MouseRight]", target: mine[0] });
    await user.click(within(await screen.findByRole("menu")).getByRole("menuitem", { name: "Remove from this playlist" }));
    await waitFor(() => expect(toasts()).toContain("Removed from this playlist"));
    await waitFor(async () => expect((await backend.collabPage(MOCK_SHARED_ID)).rows.length).toBe(before - 1));
  });

  it("hides Remove from this playlist from listeners and leaves their playlists out of Add to playlist", async () => {
    await backend.circleJoin(MOCK_COLLAB_INVITE);
    const view = await backend.circleState();
    const listenerOne = view.playlists.find((p) => p.role === "listener")!;
    const user = userEvent.setup();
    render(
      <>
        <SharedPlaylistPage id={listenerOne.id} />
        <Hosts />
      </>,
    );
    await screen.findByRole("heading", { name: listenerOne.title });
    const row = (await screen.findAllByRole("row")).find((r) => r.hasAttribute("data-track-id"))!;
    await user.pointer({ keys: "[MouseRight]", target: row });
    expect(within(await screen.findByRole("menu")).queryByRole("menuitem", { name: "Remove from this playlist" })).toBeNull();
    const targets = editablePlaylists(collaboratorPlaylists(view.playlists), "");
    expect(targets.map((p) => p.id)).toContain(MOCK_SHARED_ID);
    expect(targets.map((p) => p.id)).not.toContain(listenerOne.id);
  });

  it("asks before leaving and says what leaving means", async () => {
    const user = userEvent.setup();
    page();
    await user.click(await screen.findByRole("button", { name: "More options for Road Trip" }));
    await user.click(await screen.findByRole("menuitem", { name: "Leave playlist" }));
    const dialog = await screen.findByRole("dialog", { name: "Are you sure?" });
    expect(within(dialog).getByText(/You'll lose access to Road Trip/)).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Not now" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    await user.click(screen.getByRole("button", { name: "More options for Road Trip" }));
    await user.click(await screen.findByRole("menuitem", { name: "Leave playlist" }));
    await user.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Leave playlist" }));
    await waitFor(() => expect(toasts()).toContain("You left Road Trip"));
    await waitFor(() => expect(useCircle.getState().view.playlists.some((p) => p.id === MOCK_SHARED_ID)).toBe(false));
  });

  it("offers Leave playlist on your own role in the permissions", async () => {
    const user = userEvent.setup();
    page();
    await user.click(await screen.findByRole("button", { name: /Playlist permissions$/ }));
    const dialog = await screen.findByRole("dialog", { name: "Playlist permissions" });
    expect(within(dialog).getByText("listener (you)")).toBeInTheDocument();
    // Only the owner changes who's private or public.
    expect(within(dialog).getByRole("radio", { name: /Public/ })).toBeDisabled();
    await user.click(within(dialog).getByRole("button", { name: "Your role: Collaborator" }));
    await user.click(await screen.findByRole("menuitem", { name: "Leave playlist" }));
    expect(await screen.findByRole("dialog", { name: "Are you sure?" })).toBeInTheDocument();
  });

  it("removing it from your library keeps you on it", async () => {
    const user = userEvent.setup();
    page();
    await user.click(await screen.findByRole("button", { name: "More options for Road Trip" }));
    await user.click(await screen.findByRole("menuitem", { name: "Remove from Your Library" }));
    await waitFor(() => expect(toasts()).toContain("Removed from Your Library"));
    const s = (await backend.circleState()).playlists.find((p) => p.id === MOCK_SHARED_ID)!;
    expect([s.inLibrary, s.role]).toEqual([false, "collaborator"]);
  });

  it("says when the owner deleted it", async () => {
    page();
    await screen.findByRole("heading", { name: "Road Trip" });
    act(() => mock.circle.ownerDeletes());
    expect(await screen.findByText(/isn't available any more/)).toBeInTheDocument();
    await waitFor(() => expect(toasts()).toContain("Morgan deleted Road Trip."));
  });

  it("keeps your edits while the owner is unreachable and sends them when they're back", async () => {
    page();
    await screen.findByRole("heading", { name: "Road Trip" });
    act(() => mock.circle.ownerOffline(true));
    await act(async () => backend.collabAdd(MOCK_SHARED_ID, [mock.fx.tracks[2].id]));
    expect(await screen.findByText(/hasn't answered/)).toBeInTheDocument();
    expect(screen.getByText(/On the way to Morgan/)).toBeInTheDocument();
    act(() => mock.circle.ownerOffline(false));
    await waitFor(() => expect(screen.queryByText(/On the way to Morgan/)).toBeNull());
    expect(screen.queryByText(/hasn't answered/)).toBeNull();
  });
});

describe("Blends", () => {
  it("shows the members, Made for you, Added by, and hides a song for good", async () => {
    const user = userEvent.setup();
    render(
      <>
        <BlendPage id={MOCK_BLEND_ID} />
        <Hosts />
      </>,
    );
    expect(await screen.findByRole("heading", { name: "listener + Robin + Jules" })).toBeInTheDocument();
    expect(screen.getByText("A Blend of music for listener, Robin, and Jules")).toBeInTheDocument();
    expect(screen.getByText("Made for listener")).toBeInTheDocument();
    expect(screen.getAllByRole("columnheader", { name: "Added by" })[0]).toBeInTheDocument();
    // Taking turns.
    await waitFor(() => expect(document.querySelectorAll(".track-row[data-row-key]").length).toBe(50));
    const by = Array.from(document.querySelectorAll(".track-row[data-row-key] .track-row__by .truncate")).map((e) => e.textContent);
    expect(by.slice(0, 3)).toEqual(["listener", "Robin", "Jules"]);
    expect(screen.queryByText(/match/i)).toBeNull();
    const first = document.querySelector<HTMLElement>(".track-row[data-row-key]")!;
    const title = first.querySelector(".track-row__title")!.textContent!;
    const tid = first.dataset.trackId!;
    const shown = () => !!document.querySelector(`.track-row[data-track-id="${tid}"]`);
    await user.click(within(first).getByRole("button", { name: `Hide ${title} in this playlist` }));
    await waitFor(() => expect(toasts()).toContain(`Hidden in this Blend: ${title}`));
    expect(shown()).toBe(false);
    // Made again: it doesn't come back.
    await user.click(screen.getByRole("button", { name: /^More options for listener/ }));
    await user.click(await screen.findByRole("menuitem", { name: "Refresh" }));
    await waitFor(async () => expect((await backend.blendPage(MOCK_BLEND_ID)).rows.some((r) => r.track.id === tid)).toBe(false));
    expect(shown()).toBe(false);
    expect(screen.getByRole("region", { name: "Not in your library" })).toBeInTheDocument();
  });

  it("has the made-for-you menu plus Leave Blend, which asks first", async () => {
    const user = userEvent.setup();
    render(
      <>
        <BlendPage id={MOCK_BLEND_ID} />
        <Hosts />
      </>,
    );
    await user.click(await screen.findByRole("button", { name: /^More options for listener/ }));
    const menu = await screen.findByRole("menu");
    for (const item of ["Add to queue", "Save as playlist", "Refresh", "Leave Blend"]) expect(within(menu).getByRole("menuitem", { name: item })).toBeInTheDocument();
    await user.click(within(menu).getByRole("menuitem", { name: "Leave Blend" }));
    await user.click(within(await screen.findByRole("dialog", { name: "Leave Blend?" })).getByRole("button", { name: "Leave Blend" }));
    await waitFor(() => expect(useCircle.getState().view.blends).toHaveLength(0));
  });

  it("is in the Create menu between the playlist entries and Folder", async () => {
    const user = userEvent.setup();
    render(
      <>
        <Sidebar collapsed={false} />
        <ContextMenuHost />
      </>,
    );
    await user.click(await screen.findByRole("button", { name: "Create playlist or folder" }));
    const items = within(await screen.findByRole("menu")).getAllByRole("menuitem").map((m) => m.textContent);
    const at = items.findIndex((t) => t?.startsWith("Blend"));
    expect(items[at]).toBe("BlendCombine friends' tastes into a playlist");
    expect(items[at + 1]).toBe("Folder");
    expect(items[at - 1]).toMatch(/playlist/i);
    await user.click(within(screen.getByRole("menu")).getAllByRole("menuitem")[at]);
    expect(route()).toEqual({ name: "blendNew" });
  });

  it("creates a Blend with people picked directly, ten people at most, and a link for anyone else", async () => {
    const user = userEvent.setup();
    render(
      <>
        <BlendCreatePage />
        <Hosts />
      </>,
    );
    const sam = await screen.findByRole("button", { name: /Sam/ });
    expect(screen.getByText("1 of 10 people, you included")).toBeInTheDocument();
    await user.click(sam);
    expect(sam).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByText("2 of 10 people, you included")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Create Blend" }));
    await waitFor(() => expect(toasts()).toContain("Invited 1 person"));
    expect(await screen.findByText(/^platter:\/\/blend\//)).toBeInTheDocument();
    await waitFor(() => expect(useCircle.getState().view.blends.some((b) => b.title === "listener + Sam")).toBe(true));
    await user.click(screen.getByRole("button", { name: "Open Blend" }));
    expect(route()).toMatchObject({ name: "playlist" });
    await expect(backend.blendCreate(Array.from({ length: 10 }, (_, i) => `k${i}`))).rejects.toMatchObject({ message: expect.stringMatching(/10 people/) });
  });
});

describe("joining and failure states", () => {
  it("says an invite expired or was used, and the relay is down", async () => {
    mock.circle.failNextJoin("expired");
    expect(await joinCircle(MOCK_COLLAB_INVITE)).toBeNull();
    expect(toasts()).toContain("That invite has expired. Ask for a new one.");
    mock.circle.failNextJoin("used");
    await joinCircle(MOCK_COLLAB_INVITE);
    expect(toasts()).toContain("That invite was already used. Ask for a new one.");
    act(() => mock.circle.relay("unreachable"));
    await joinCircle(MOCK_COLLAB_INVITE);
    expect(toasts()).toContain("Couldn't reach the relay to join. Try again in a while.");
    await expect(backend.collabInviteLink(unsharedId, "listener")).rejects.toMatchObject({ kind: "network" });
  });

  it("waits for an owner who's offline, and opens the playlist once in", async () => {
    mock.circle.joinWaits(true);
    const r = await joinCircle(MOCK_COLLAB_INVITE);
    expect(r?.state).toBe("waiting");
    expect(toasts()).toContain("Asked to join. You're in once the owner is online.");
    mock.circle.joinWaits(false);
    const ok = await joinCircle(MOCK_COLLAB_INVITE);
    expect(route()).toEqual({ name: "playlist", id: ok?.state === "joined" ? ok.id : "" });
  });

  it("shows an invite that came straight to you with Join and Not now", async () => {
    const user = userEvent.setup();
    render(<Hosts />);
    act(() => mock.circle.ring("Robin", "playlist", "Summer Nights"));
    const card = await screen.findByRole("group", { name: "Invite from Robin" });
    expect(card).toHaveTextContent("Robin invited you to Summer Nights");
    await user.click(within(card).getByRole("button", { name: "Join" }));
    await waitFor(() => expect(route()).toMatchObject({ name: "playlist" }));
    expect(screen.queryByRole("group", { name: "Invite from Robin" })).toBeNull();
  });

  it("needs the setting on", async () => {
    await mock.settingsSet({ collabPlaylists: false });
    await expect(backend.circleJoin(MOCK_COLLAB_INVITE)).rejects.toMatchObject({ message: "Collaborative playlists are off in Settings." });
  });
});

describe("words", () => {
  const s = (o: Partial<SharedSummary>) => ({ title: "Road Trip", public: false, role: "listener" as const, ...o });
  it("says what leaving means in each case", () => {
    expect(leaveMessage(s({}))).toMatch(/lose access/);
    expect(leaveMessage(s({ public: true, role: "collaborator" }))).toMatch(/can't add any more/);
    expect(leaveMessage(s({ public: true }))).toMatch(/lose access if it's made private/);
  });

  it("names the owner and one member, or counts the others", () => {
    const p = (name: string, you = false) => ({ id: name, name, avatar: null, role: null, you, added: 0 });
    const base = { people: [p("Morgan"), p("Casey")] } as SharedSummary;
    expect(bylineText(base)).toBe("Morgan and Casey");
    expect(bylineText({ ...base, people: [p("Morgan"), p("Casey"), p("Avery"), p("listener", true)] })).toBe("Morgan and 3 others");
    expect(bylineText({ ...base, people: [p("listener", true), p("Casey")] })).toBe("You and Casey");
  });
});
