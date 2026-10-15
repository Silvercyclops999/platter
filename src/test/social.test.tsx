/**
 * Social: the friend activity panel (rows, short times, hover play, the
 * owner-only and offline states), the Follow pill and the Followed chip,
 * What's New with its New and Earlier groups and the bell's dot, the
 * listening session strip (host, guest, host gone, ended), joining from an
 * invite link, the session card, and the Settings rows.
 */
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { ContextMenuHost } from "../components/ContextMenu";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { MOCK_FRIENDS, MOCK_INVITE } from "../lib/mock/social";
import { shortAgo } from "../lib/relativeTime";
import { DEFAULT_SETTINGS } from "../lib/types";
import { ArtistPage } from "../pages/ArtistPage";
import { SettingsPage } from "../pages/SettingsPage";
import { WhatsNewPage } from "../pages/WhatsNewPage";
import { FriendActivityPanel } from "../shell/FriendActivityPanel";
import { ListenStrip, SessionCard, namesLine } from "../shell/ListenStrip";
import { Sidebar } from "../shell/Sidebar";
import { TopBar } from "../shell/TopBar";
import { useApp } from "../stores/app";
import { useLibraryView } from "../stores/library";
import { useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import { openDeepLink } from "../stores/share";
import { resetSocial, startSocial, useSocial } from "../stores/social";
import { useToasts } from "../stores/toasts";
import { useUi } from "../stores/ui";
import { listenAlong } from "../stores/social";

// Sessions are off in the app until the relay transport lands; keep covering them.
listenAlong.enabled = true;

const mock = backend as MockBackend;
const toasts = () => useToasts.getState().toasts.map((t) => t.message);
const route = () => useRouter.getState().entries[useRouter.getState().index].route;

beforeEach(async () => {
  mock.social.reset();
  await mock.settingsSet({ ...DEFAULT_SETTINGS });
  useSettings.setState({ settings: null });
  await loadSettings();
  useApp.setState({ auth: { state: "signedIn", username: "listener", needsServerPick: false } });
  useToasts.setState({ toasts: [] });
  useUi.setState({ menu: null, activityOpen: false, queueOpen: false, nowPlayingOpen: false, outputOpen: false });
  useRouter.getState().reset({ name: "home" });
  useLibraryView.setState({ filter: "all", sub: null, downloaded: null });
  try {
    localStorage.clear();
  } catch {
    /* none */
  }
  resetSocial();
  await startSocial();
});

describe("short times", () => {
  it("say now, minutes, hours and days", () => {
    const now = 1_000_000;
    expect(shortAgo(now - 5, now)).toBe("now");
    expect(shortAgo(now - 3600 * 5, now, true)).toBe("now");
    expect(shortAgo(now - 11 * 60, now)).toBe("11 min");
    expect(shortAgo(now - 2 * 3600 - 59, now)).toBe("2 hr");
    expect(shortAgo(now - 27 * 3600, now)).toBe("1 d");
  });

  it("list who's listening in a sentence", () => {
    expect(namesLine(["A"])).toBe("A");
    expect(namesLine(["A", "B"])).toBe("A and B");
    expect(namesLine(["A", "B", "C"])).toBe("A, B and C");
    expect(namesLine(["A", "B", "C", "D", "E"])).toBe("A, B and 3 more");
  });
});

describe("friend activity", () => {
  it("opens from the top bar into the right panel and closes again", async () => {
    render(<TopBar />);
    const btn = screen.getByRole("button", { name: "Friend activity" });
    expect(btn).toHaveAttribute("aria-pressed", "false");
    await userEvent.click(btn);
    expect(useUi.getState().activityOpen).toBe(true);
    expect(btn).toHaveAttribute("aria-pressed", "true");
    // It takes the slot from the queue, and the queue takes it back.
    act(() => useUi.getState().setQueueOpen(true));
    expect(useUi.getState().activityOpen).toBe(false);
  });

  it("shows one row per person, newest first, with name, song, album and a short time", async () => {
    render(<FriendActivityPanel />);
    const list = await screen.findByRole("list", { name: "Friends' latest songs" });
    const rows = within(list).getAllByRole("listitem");
    expect(rows.map((r) => r.querySelector(".friend-row__name")?.textContent)).toEqual(MOCK_FRIENDS);
    expect(rows.map((r) => r.querySelector(".friend-row__when")?.textContent)).toEqual(["now", "11 min", "2 hr", "1 d"]);
    expect(rows[0]).toHaveAccessibleName(/listening now/);
    const panel = (await mock.friendActivity()).friends[1];
    expect(rows[1]).toHaveTextContent(panel.title);
    expect(rows[1]).toHaveTextContent(panel.album);
    // Hovering the avatar offers to play that song here.
    await userEvent.click(within(rows[1]).getByRole("button", { name: `Play ${panel.title}` }));
    await waitFor(async () => expect((await mock.playerState()).track?.id).toBe(panel.trackId));
    // The song opens its album.
    await userEvent.click(within(rows[1]).getAllByRole("button", { name: panel.title })[0]);
    expect(route()).toEqual({ name: "album", id: panel.albumId });
  });

  it("tells someone who doesn't own the server that activity isn't available", async () => {
    mock.social.access = "notAdmin";
    render(<FriendActivityPanel />);
    expect(await screen.findByRole("status")).toHaveTextContent("Friend activity isn't available on this server. Plex only shares who's listening with the server's owner.");
  });

  it("says when the server can't be reached", async () => {
    mock.social.access = "offline";
    render(<FriendActivityPanel />);
    expect(await screen.findByRole("status")).toHaveTextContent("Can't reach your server right now.");
  });

  it("hides the top bar button when the setting is off", async () => {
    await act(async () => {
      useSettings.setState({ settings: { ...DEFAULT_SETTINGS, showFriendActivity: false } });
    });
    render(<TopBar />);
    expect(screen.queryByRole("button", { name: "Friend activity" })).toBeNull();
  });
});

describe("following artists", () => {
  it("the pill follows and unfollows, the menu too, and Followed narrows the library's artists", async () => {
    const artist = mock.fx.artists[5];
    render(
      <>
        <ArtistPage id={artist.id} />
        <ContextMenuHost />
      </>,
    );
    const pill = await screen.findByRole("button", { name: `Follow ${artist.title}` });
    expect(pill).toHaveTextContent("Follow");
    expect(pill).toHaveAttribute("aria-pressed", "false");
    await userEvent.click(pill);
    const following = await screen.findByRole("button", { name: `Following ${artist.title}` });
    expect(following).toHaveTextContent("Following");
    expect((await mock.followsList()).some((f) => f.id === artist.id)).toBe(true);
    // Unfollow from the ⋯ menu.
    await userEvent.click(screen.getByRole("button", { name: `More options for ${artist.title}` }));
    await userEvent.click(await screen.findByRole("menuitem", { name: "Unfollow" }));
    await screen.findByRole("button", { name: `Follow ${artist.title}` });
    expect((await mock.followsList()).some((f) => f.id === artist.id)).toBe(false);
  });

  it("puts a Followed chip under Artists in the library", async () => {
    render(<Sidebar collapsed={false} />);
    await userEvent.click(await screen.findByRole("button", { name: "Artists" }));
    const chip = await screen.findByRole("button", { name: "Followed" });
    await userEvent.click(chip);
    expect(chip).toHaveAttribute("aria-pressed", "true");
    const followed = useSocial.getState().follows.map((f) => f.title);
    await waitFor(() => {
      const shown = screen.getAllByRole("listitem").map((li) => li.textContent ?? "");
      for (const name of followed) expect(shown.some((t) => t.includes(name))).toBe(true);
      expect(shown.filter((t) => !t.includes("Liked Songs")).length).toBe(followed.length);
    });
  });
});

describe("What's New", () => {
  it("the bell has a dot for unseen releases that goes once the page is seen", async () => {
    render(<TopBar />);
    const bell = screen.getByRole("button", { name: /^What's New/ });
    await waitFor(() => expect(screen.getByTestId("whats-new-dot")).toBeInTheDocument());
    expect(bell).toHaveAccessibleName("What's New, 2 new");
    await userEvent.click(bell);
    expect(route()).toEqual({ name: "whatsNew" });
    render(<WhatsNewPage />);
    await screen.findByRole("heading", { name: "New" });
    await waitFor(() => expect(screen.queryByTestId("whats-new-dot")).toBeNull());
    // Announcements off: no dot even with something new.
    await act(async () => {
      useSettings.setState({ settings: { ...DEFAULT_SETTINGS, whatsNewAnnouncements: false } });
      mock.social.addRelease(mock.fx.albums[9].id);
    });
    expect(screen.queryByTestId("whats-new-dot")).toBeNull();
  });

  it("groups releases into New and Earlier with type, when, play and options", async () => {
    render(
      <>
        <WhatsNewPage />
        <ContextMenuHost />
      </>,
    );
    const fresh = await screen.findByRole("region", { name: "New" });
    const earlier = screen.getByRole("region", { name: "Earlier" });
    expect(within(fresh).getAllByRole("listitem")).toHaveLength(2);
    expect(within(earlier).getAllByRole("listitem").length).toBeGreaterThan(0);
    const first = within(fresh).getAllByRole("listitem")[0];
    expect(first.querySelector(".release-row__meta")?.textContent).toMatch(/^(Album|EP|Single) • /);
    const title = first.querySelector(".release-row__title")!.textContent!;
    await userEvent.click(within(first).getByRole("button", { name: `Play ${title}` }));
    await waitFor(async () => expect((await mock.playerState()).queue.context?.kind).toBe("album"));
    await userEvent.click(within(first).getByRole("button", { name: `More options for ${title}` }));
    expect(await screen.findByRole("menuitem", { name: "Unfollow" })).toBeInTheDocument();
    expect(screen.getByRole("menuitem", { name: "Start a listening session" })).toBeInTheDocument();
  });

  it("without follows, lists what's new in the library and says how to narrow it", async () => {
    for (const f of await mock.followsList()) await mock.followSet(f, false);
    render(<WhatsNewPage />);
    expect(await screen.findByText("New in your library. Follow artists to see only their releases here.")).toBeInTheDocument();
  });
});

describe("listening sessions", () => {
  it("the host starts from the queue card, copies the invite, sees who joins, and ends it", async () => {
    const copied: string[] = [];
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText: (t: string) => (copied.push(t), Promise.resolve()) },
      configurable: true,
    });
    render(
      <>
        <SessionCard dismissable />
        <ListenStrip />
      </>,
    );
    await userEvent.click(screen.getByRole("button", { name: "Start a listening session" }));
    expect(await screen.findByRole("region", { name: "Listening session" })).toHaveTextContent("waiting for people to join");
    expect(useSocial.getState().listen.invite).toBe(MOCK_INVITE);
    await waitFor(() => expect(toasts()).toContain("Listening session started. The invite link is copied."));
    expect(copied).toEqual([MOCK_INVITE]);
    // The card goes while in a session.
    expect(screen.queryByRole("group", { name: "Listen along" })).toBeNull();
    act(() => mock.social.guestJoins("Casey"));
    await waitFor(() => expect(screen.getByRole("region", { name: "Listening session" })).toHaveTextContent("Casey listening along"));
    expect(screen.getByRole("button", { name: "Copy invite link" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "End session" }));
    await waitFor(() => expect(screen.queryByRole("region", { name: "Listening session" })).toBeNull());
    expect(toasts()).toContain("Listening session ended");
  });

  it("the queue card can be dismissed for good", async () => {
    const { unmount } = render(<SessionCard dismissable />);
    await userEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(screen.queryByRole("group", { name: "Listen along" })).toBeNull();
    unmount();
    render(<SessionCard dismissable />);
    expect(screen.queryByRole("group", { name: "Listen along" })).toBeNull();
  });

  it("an invite link joins, and a host that goes away or ends it is said so", async () => {
    render(<ListenStrip />);
    await act(() => openDeepLink({ status: "open", kind: null, id: null, albumId: null, atSeconds: null, join: MOCK_INVITE }));
    const strip = await screen.findByRole("region", { name: "Listening session" });
    expect(strip).toHaveTextContent(`Listening along with ${MOCK_FRIENDS[1]}, ${MOCK_FRIENDS[2]}`);
    expect(toasts()).toContain(`Listening along with ${MOCK_FRIENDS[1]}`);
    expect(within(strip).getByRole("button", { name: "Leave" })).toBeInTheDocument();

    act(() => mock.social.hostGone());
    await waitFor(() => expect(strip).toHaveTextContent(`${MOCK_FRIENDS[1]} stopped answering`));
    expect(toasts()).toContain(`${MOCK_FRIENDS[1]} stopped answering. You're no longer listening along.`);
    expect(within(strip).getByRole("button", { name: "Close" })).toBeInTheDocument();
    await userEvent.click(within(strip).getByRole("button", { name: "Close" }));
    await waitFor(() => expect(screen.queryByRole("region", { name: "Listening session" })).toBeNull());

    await act(() => openDeepLink({ status: "open", kind: null, id: null, albumId: null, atSeconds: null, join: MOCK_INVITE }));
    act(() => mock.social.hostEnds());
    await waitFor(() => expect(screen.getByRole("region", { name: "Listening session" })).toHaveTextContent("ended the listening session"));
    expect(toasts()).toContain("The listening session ended.");
  });

  it("a join that can't reach the host says why", async () => {
    mock.social.failNextJoin = "Couldn't reach the host. You need to be on the same network.";
    await openDeepLink({ status: "open", kind: null, id: null, albumId: null, atSeconds: null, join: MOCK_INVITE });
    expect(toasts()).toContain("Couldn't reach the host. You need to be on the same network.");
    expect(useSocial.getState().listen.role).toBe("none");
  });

  it("guests add to the session's queue when the host lets them", async () => {
    await act(() => openDeepLink({ status: "open", kind: null, id: null, albumId: null, atSeconds: null, join: MOCK_INVITE }));
    const t = mock.fx.tracks[3];
    const { addToSession } = await import("../stores/social");
    await addToSession([t.id]);
    expect(toasts()).toContain("Added to the session's queue");
    await waitFor(async () => expect((await mock.playerState()).queue.userQueue.some((i) => i.track.id === t.id)).toBe(true));
  });
});

describe("settings", () => {
  it("has the friend activity, following and session rows with their defaults", async () => {
    render(<SettingsPage />);
    const social = await screen.findByRole("region", { name: "Social" });
    const sw = (name: string) => within(social).getByRole("switch", { name });
    await waitFor(() => expect(sw("Show friend activity")).toBeChecked());
    expect(sw("Share my activity with Platter friends")).toBeChecked();
    expect(sw("What's New announcements")).toBeChecked();
    expect(sw("Notify me about new releases")).not.toBeChecked();
    expect(sw("Let guests add to the queue")).toBeChecked();
    await userEvent.click(sw("Notify me about new releases"));
    await waitFor(async () => expect((await mock.settingsGet()).notifyNewReleases).toBe(true));
    expect(social).toHaveTextContent("Only the server's owner can see it");
    expect(social).toHaveTextContent("in a private session");
  });
});
