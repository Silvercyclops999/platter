/**
 * Sharing your music through Plex: "<name> can't hear this" with Let them
 * listen from my server (one click, or the dialog the first time), the
 * unreleased count and Plex Pass stop, the guest's invite card and accept,
 * Settings, Sharing (share by username, a link, people with access,
 * remove), and the failure states. All against the mock core.
 */
import { ONLINE_EXTRAS } from "./onlineExtras";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { MOCK_INVITE } from "../lib/mock/social";
import { DEFAULT_SETTINGS } from "../lib/types";
import { SettingsPage } from "../pages/SettingsPage";
import { ListenMark, ListenStrip } from "../shell/ListenStrip";
import { ShareMusicDialog } from "../shell/ShareMusic";
import { loadSettings, useSettings } from "../stores/settings";
import { confirmLink, openDeepLink } from "../stores/share";
import { resetSharing, startSharing, useSharing } from "../stores/sharing";
import { listenAlong, resetSocial, startSocial } from "../stores/social";
import { useToasts } from "../stores/toasts";
import { useRouter } from "../stores/router";
import { useApp } from "../stores/app";

listenAlong.enabled = true;

const mock = backend as MockBackend;
const toasts = () => useToasts.getState().toasts.map((t) => t.message);

beforeEach(async () => {
  mock.social.reset();
  mock.sharing.reset();
  await mock.settingsSet({ ...DEFAULT_SETTINGS, ...ONLINE_EXTRAS });
  useSettings.setState({ settings: null });
  await loadSettings();
  useApp.setState({ auth: { state: "signedIn", username: "listener", needsServerPick: false } });
  useToasts.setState({ toasts: [] });
  useRouter.getState().reset({ name: "home" });
  resetSocial();
  resetSharing();
  await startSocial();
  await startSharing();
});

function session() {
  return render(
    <>
      <ListenStrip />
      <ListenMark />
      <ShareMusicDialog />
    </>,
  );
}

/** Hosting, with Robin who can't hear the song. */
async function hostingRobin() {
  session();
  act(() => {
    mock.social.start();
    mock.social.guestJoins("Robin");
    mock.social.guestCantHear("Robin", true);
  });
  const callout = await screen.findByRole("group", { name: "People who can't hear this" });
  return callout;
}

describe("Let them listen from my server", () => {
  it("the first time, picks libraries in the dialog, sees the unreleased count, and invites", async () => {
    const callout = await hostingRobin();
    expect(callout).toHaveTextContent("Robin can't hear this");
    expect(callout).toHaveTextContent("It isn't on any server they can reach.");
    await userEvent.click(within(callout).getByRole("button", { name: "Let them listen from my server" }));
    // No default libraries yet: the dialog asks.
    const dialog = await screen.findByRole("dialog", { name: "Let Robin listen from your server" });
    const libs = await within(dialog).findByRole("group", { name: "Libraries" });
    expect(within(libs).getAllByRole("checkbox").map((c) => c.closest("label")?.textContent)).toEqual([
      "Music4 unreleased albums",
      "Live TapesNothing unreleased",
      "Old RipsNot read by Platter yet",
    ]);
    expect(within(dialog).getByRole("button", { name: "Invite" })).toBeDisabled();
    await userEvent.click(within(libs).getByRole("checkbox", { name: /^Music/ }));
    expect(within(dialog).getByText("What you picked holds 4 unreleased albums.")).toBeInTheDocument();
    expect(within(dialog).getByRole("switch", { name: "Hide my unreleased music from people I share with" })).toBeChecked();
    expect(within(dialog).getByText(/Platter labels them in Plex/)).toBeInTheDocument();
    await userEvent.click(within(dialog).getByRole("button", { name: "Invite" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    // The choice is the default from now on.
    expect(useSettings.getState().settings?.shareDefaultLibraries).toEqual(["3"]);
    await waitFor(() => expect(screen.getByText("Invited. Robin accepts in their Platter.")).toBeInTheDocument(), { timeout: 3000 });
    expect(mock.sharing.shared).toEqual([{ invitee: "robin_on_plex", options: { libraries: ["3"], hideUnreleased: true, allowDownloads: false }, excludesLabel: true }]);
    expect(mock.sharing.labelled).toBe(4);
    // They accept and hear your copy: the card goes.
    act(() => mock.social.guestAccepts("Robin"));
    await waitFor(() => expect(screen.queryByRole("group", { name: "People who can't hear this" })).toBeNull());
  });

  it("is one click once default libraries are chosen", async () => {
    await mock.settingsSet({ shareDefaultLibraries: ["7"] });
    await loadSettings();
    const callout = await hostingRobin();
    await userEvent.click(within(callout).getByRole("button", { name: "Let them listen from my server" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    await waitFor(() => expect(mock.sharing.shared.map((x) => x.options.libraries)).toEqual([["7"]]), { timeout: 3000 });
  });

  it("without Plex Pass, nothing is shared until the host picks: without the library, or cancel", async () => {
    mock.sharing.plexPass = false;
    const callout = await hostingRobin();
    await userEvent.click(within(callout).getByRole("button", { name: "Let them listen from my server" }));
    const dialog = await screen.findByRole("dialog");
    const libs = await within(dialog).findByRole("group", { name: "Libraries" });
    await userEvent.click(within(libs).getByRole("checkbox", { name: /^Music/ }));
    const stop = within(dialog).getByRole("alert");
    expect(stop).toHaveTextContent("Keeping unreleased music out needs Plex Pass on your account, and it doesn't have it. Nothing is shared until you choose.");
    expect(within(dialog).getByRole("button", { name: "Invite" })).toBeDisabled();
    // With a library that holds nothing unreleased too, it can go without Music.
    await userEvent.click(within(libs).getByRole("checkbox", { name: /^Live Tapes/ }));
    await userEvent.click(within(dialog).getByRole("button", { name: "Share without Music" }));
    await waitFor(() => expect(mock.sharing.shared.map((x) => x.options.libraries)).toEqual([["7"]]), { timeout: 3000 });
    expect(mock.sharing.labelled).toBe(0);
    // And Cancel shares nothing.
    act(() => mock.social.guestJoins("Casey"));
    act(() => mock.social.guestCantHear("Casey", true));
    await mock.settingsSet({ shareDefaultLibraries: [] });
    await loadSettings();
    const casey = await screen.findByText("Casey can't hear this", { selector: ".share-callout__title" });
    await userEvent.click(within(casey.closest(".share-callout") as HTMLElement).getByRole("button", { name: "Let them listen from my server" }));
    const d2 = await screen.findByRole("dialog");
    await userEvent.click(within(await within(d2).findByRole("group", { name: "Libraries" })).getByRole("checkbox", { name: /^Music/ }));
    await userEvent.click(within(within(d2).getByRole("alert")).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(mock.sharing.shared).toHaveLength(1);
  });

  it("says when the guest doesn't take invites in Platter, and when Plex fails", async () => {
    await mock.settingsSet({ shareDefaultLibraries: ["7"] });
    await loadSettings();
    mock.social.guestTakesInvites(false);
    const callout = await hostingRobin();
    await userEvent.click(within(callout).getByRole("button", { name: "Let them listen from my server" }));
    await waitFor(() => expect(callout).toHaveTextContent("Robin's Platter doesn't take invites. Share from Settings, Sharing with their Plex username."), { timeout: 3000 });
    mock.social.guestTakesInvites(true);
    mock.sharing.failNext = "plexDown";
    await userEvent.click(within(callout).getByRole("button", { name: "Try again" }));
    await waitFor(() => expect(callout).toHaveTextContent("Plex isn't answering right now. Try again in a while."), { timeout: 3000 });
    mock.sharing.failNext = "alreadyShared";
    await userEvent.click(within(callout).getByRole("button", { name: "Try again" }));
    await waitFor(() => expect(within(callout).getByLabelText("Invited")).toBeInTheDocument(), { timeout: 3000 });
  });

  it("isn't offered when the setting is off", async () => {
    await mock.settingsSet({ shareOfferServer: false });
    await loadSettings();
    session();
    act(() => {
      mock.social.start();
      mock.social.guestJoins("Robin");
      mock.social.guestCantHear("Robin", true);
    });
    await waitFor(() => expect(screen.getAllByText(/Robin can't hear this/).length).toBeGreaterThan(0));
    expect(screen.queryByRole("button", { name: "Let them listen from my server" })).toBeNull();
  });
});

describe("the guest's invite", () => {
  const join = () =>
    act(async () => {
      await openDeepLink({ status: "open", kind: null, id: null, albumId: null, atSeconds: null, join: MOCK_INVITE });
      await confirmLink();
    });

  it("shows as a card in Listen along, and accepting plays the host's copy", async () => {
    session();
    await join();
    act(() => mock.social.songNotHere("Basement Tapes No. 4"));
    await waitFor(() => expect(screen.getAllByText(/Not in your library: Basement Tapes No. 4/).length).toBeGreaterThan(0));
    act(() => mock.social.hostSharesServer());
    const card = await screen.findByRole("group", { name: "Invite from Jules" });
    expect(card).toHaveTextContent("Jules shared their music with you");
    expect(card).toHaveTextContent("Accept to hear Jules's copy of this song and the rest of the session.");
    expect(toasts()).toContain("Jules shared their music with you");
    await userEvent.click(within(card).getByRole("button", { name: "Accept" }));
    await waitFor(() => expect(screen.queryByRole("group", { name: "Invite from Jules" })).toBeNull());
    await waitFor(() => expect(screen.queryByText(/Not in your library/)).toBeNull());
    expect(toasts()).toContain("Added Jules's Music. Their music joins your library as it's read.");
  });

  it("stays out of the way with Accept invites inside Platter off", async () => {
    await mock.settingsSet({ shareAcceptInvites: false });
    await loadSettings();
    session();
    await join();
    act(() => mock.social.hostSharesServer());
    await waitFor(() => expect(useSharing.getState().invites).toHaveLength(1));
    expect(screen.queryByRole("group", { name: "Invite from Jules" })).toBeNull();
    expect(toasts()).not.toContain("Jules shared their music with you");
  });

  it("a share link asks for someone's music, and their invite follows", async () => {
    await act(async () => {
      await openDeepLink({ status: "open", kind: null, id: null, albumId: null, atSeconds: null, shareAsk: "platter://share/AV8MKp57HUw4puL5CxfT5EXMOOHwS3stlqDkyLHT9XopaPBRgA" });
      await confirmLink();
    });
    expect(toasts()).toContain("Asked Morgan for their music. Their invite shows here in a moment.");
    await waitFor(() => expect(useSharing.getState().invites.map((i) => i.from)).toEqual(["Morgan"]), { timeout: 3000 });
  });
});

describe("Settings, Sharing", () => {
  async function sharingSection() {
    render(
      <>
        <SettingsPage />
        <ShareMusicDialog />
      </>,
    );
    return screen.findByRole("region", { name: "Sharing" });
  }

  it("someone who opened a share link waits for a yes", async () => {
    const asks = vi.spyOn(backend, "sharingLinkAsks").mockResolvedValue([{ id: "a1", user: "alex_on_plex", name: "Alex", at: 1 }]);
    const approve = vi.spyOn(backend, "sharingLinkApprove").mockResolvedValue("Shared with Alex. Plex sent them an invite.");
    const sec = await sharingSection();
    const list = await within(sec).findByRole("list", { name: "Asking for your music" });
    expect(list).toHaveTextContent("Plex username alex_on_plex");
    expect(approve).not.toHaveBeenCalled();
    asks.mockResolvedValue([]);
    await userEvent.click(within(list).getByRole("button", { name: "Share" }));
    await waitFor(() => expect(approve).toHaveBeenCalledWith("a1"));
    await waitFor(() => expect(within(sec).queryByRole("list", { name: "Asking for your music" })).toBeNull());
    expect(toasts()).toContain("Shared with Alex. Plex sent them an invite.");
    vi.restoreAllMocks();
  });

  it("shares with a Plex username, lists people with access, and removes one", async () => {
    const sec = await sharingSection();
    const people = await within(sec).findByRole("list", { name: "People with access" });
    expect(within(people).getAllByRole("listitem")).toHaveLength(2);
    expect(people).toHaveTextContent("robin_listens");
    expect(people).toHaveTextContent("Music · Last active: 2 hours ago");
    expect(people).toHaveTextContent("Unreleased hidden");
    expect(people).toHaveTextContent("Live Tapes · Invited, not accepted yet");
    await userEvent.click(within(sec).getByRole("button", { name: "Share my music" }));
    const dialog = await screen.findByRole("dialog", { name: "Share my music" });
    await userEvent.type(within(dialog).getByLabelText("Plex username or email"), "casey_plays");
    await userEvent.click(within(await within(dialog).findByRole("group", { name: "Libraries" })).getByRole("checkbox", { name: /^Music/ }));
    await userEvent.click(within(dialog).getByRole("switch", { name: "Allow downloads" }));
    await userEvent.click(within(dialog).getByRole("button", { name: "Share" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(toasts()).toContain("Plex sent casey_plays an invite. 4 unreleased albums stay hidden.");
    expect(mock.sharing.shared[0]).toEqual({ invitee: "casey_plays", options: { libraries: ["3"], hideUnreleased: true, allowDownloads: true }, excludesLabel: true });
    await waitFor(() => expect(within(sec).getByRole("list", { name: "People with access" })).toHaveTextContent("casey_plays"));
    await userEvent.click(within(sec).getByRole("button", { name: "Remove robin_listens's access" }));
    // It asks first; Not now keeps them.
    let confirm = await screen.findByRole("dialog", { name: "Remove robin_listens's access?" });
    expect(confirm).toHaveTextContent("won't be able to play music from your server");
    await userEvent.click(within(confirm).getByRole("button", { name: "Not now" }));
    expect(within(sec).getByRole("list", { name: "People with access" })).toHaveTextContent("robin_listens");
    await userEvent.click(within(sec).getByRole("button", { name: "Remove robin_listens's access" }));
    confirm = await screen.findByRole("dialog", { name: "Remove robin_listens's access?" });
    await userEvent.click(within(confirm).getByRole("button", { name: "Remove access" }));
    await waitFor(() => expect(within(sec).getByRole("list", { name: "People with access" })).not.toHaveTextContent("robin_listens"));
    expect(toasts()).toContain("robin_listens can't reach your server anymore");
  });

  it("shows Plex's refusals in the dialog: unknown user, refused, Plex down, already shared", async () => {
    const sec = await sharingSection();
    await userEvent.click(within(sec).getByRole("button", { name: "Share my music" }));
    const dialog = await screen.findByRole("dialog", { name: "Share my music" });
    await userEvent.type(within(dialog).getByLabelText("Plex username or email"), "nobody_here");
    await userEvent.click(within(await within(dialog).findByRole("group", { name: "Libraries" })).getByRole("checkbox", { name: /^Live Tapes/ }));
    for (const [fail, words] of [
      ["unknownUser", "Plex doesn't know that username or email."],
      ["refused", "Plex turned the invite down."],
      ["plexDown", "Plex isn't answering right now. Try again in a while."],
    ] as const) {
      mock.sharing.failNext = fail;
      await userEvent.click(within(dialog).getByRole("button", { name: "Share" }));
      await waitFor(() => expect(within(dialog).getByRole("alert")).toHaveTextContent(words));
    }
    expect(mock.sharing.shared).toHaveLength(0);
    mock.sharing.failNext = "alreadyShared";
    await userEvent.click(within(dialog).getByRole("button", { name: "Share" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(toasts()).toContain("nobody_here already has access to your server.");
  });

  it("makes a one-time link with a QR code", async () => {
    const sec = await sharingSection();
    await userEvent.click(within(sec).getByRole("button", { name: "Share my music" }));
    const dialog = await screen.findByRole("dialog", { name: "Share my music" });
    await userEvent.click(within(await within(dialog).findByRole("group", { name: "Libraries" })).getByRole("checkbox", { name: /^Live Tapes/ }));
    await userEvent.click(within(dialog).getByRole("button", { name: "Make a link instead" }));
    expect(await within(dialog).findByRole("textbox", { name: "Share link" })).toHaveValue("platter://share/AV8MKp57HUw4puL5CxfT5EXMOOHwS3stlqDkyLHT9XopaPBRgA");
    expect(within(dialog).getByRole("img", { name: "QR code for the share link" })).toBeInTheDocument();
    expect(within(dialog).getByText("Works for one person, once. Keep Platter open until they've used it.")).toBeInTheDocument();
  });

  it("a library Platter hasn't read can't be counted, so it isn't shared with hiding on", async () => {
    const sec = await sharingSection();
    await userEvent.click(within(sec).getByRole("button", { name: "Share my music" }));
    const dialog = await screen.findByRole("dialog", { name: "Share my music" });
    await userEvent.type(within(dialog).getByLabelText("Plex username or email"), "robin");
    await userEvent.click(within(await within(dialog).findByRole("group", { name: "Libraries" })).getByRole("checkbox", { name: /^Old Rips/ }));
    expect(within(dialog).getByText("Platter hasn't read every library you picked, so it can't count their unreleased albums.")).toBeInTheDocument();
    await userEvent.click(within(dialog).getByRole("button", { name: "Share" }));
    await waitFor(() => expect(within(dialog).getByRole("alert")).toHaveTextContent("Platter hasn't read Old Rips yet"));
    expect(mock.sharing.shared).toHaveLength(0);
  });

  it("on a server shared with you, says only the owner can share", async () => {
    mock.sharing.owner = false;
    const sec = await sharingSection();
    expect(await within(sec).findByText("Only the server's owner can share it. Morgan's Music is shared with you.")).toBeInTheDocument();
  });

  it("lists invites to you and the sharing switches with their defaults", async () => {
    act(() => mock.sharing.inviteArrives("Casey"));
    const sec = await sharingSection();
    expect(await within(sec).findByRole("group", { name: "Invite from Casey" })).toHaveTextContent("Casey's Music joins your library. Plex may email you about it too.");
    expect(within(sec).getByRole("switch", { name: "Hide my unreleased music from people I share with" })).toBeChecked();
    expect(within(sec).getByRole("switch", { name: "Allow downloads for people I share with" })).not.toBeChecked();
    expect(within(sec).getByRole("switch", { name: "Accept invites inside Platter" })).toBeChecked();
    const defaults = await within(sec).findByRole("group", { name: "Libraries I share by default" });
    expect(within(defaults).getAllByRole("checkbox").filter((c) => (c as HTMLInputElement).checked)).toHaveLength(0);
    await userEvent.click(within(defaults).getByRole("checkbox", { name: /^Live Tapes/ }));
    await waitFor(() => expect(useSettings.getState().settings?.shareDefaultLibraries).toEqual(["7"]));
  });
});
