/**
 * Adding servers: first run offers Plex and Jellyfin, Jellyfin signs in with
 * a Quick Connect code or a password, and Settings lists every server with
 * its libraries, the primary one, sign in again and remove. Each failure
 * state is faked through the mock backend.
 */
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import App from "../App";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { EXPIRED, NOT_JELLYFIN, QUICK_CONNECT_OFF, WRONG_PASSWORD, WRONG_URL } from "../lib/mock/jellyfin";
import { DEFAULT_SETTINGS } from "../lib/types";
import { AddServerFlow } from "../pages/AddServer";
import { SettingsPage } from "../pages/SettingsPage";
import { useApp } from "../stores/app";
import { useSettings } from "../stores/settings";
import { useUi } from "../stores/ui";

const mock = backend as MockBackend;
const POLL = 10;

beforeEach(() => {
  mock.resetServers();
  useSettings.setState({ settings: { ...DEFAULT_SETTINGS } });
  useUi.getState().setSignInOpen(false);
});

function flow(props: Partial<Parameters<typeof AddServerFlow>[0]> = {}) {
  const added: string[] = [];
  render(<AddServerFlow pollMs={POLL} onDone={(s) => added.push(s.tag)} onCancel={() => {}} {...props} />);
  return added;
}

async function typeAddress(user: ReturnType<typeof userEvent.setup>, url = "jellyfin.local:8096") {
  await user.type(screen.getByLabelText("Jellyfin server address"), url);
  await user.click(screen.getByRole("button", { name: "Continue" }));
}

describe("adding a Jellyfin server", () => {
  it("shows a Quick Connect code and adds the server once it's approved", async () => {
    const user = userEvent.setup();
    mock.jellyfin.approveAfter = 2;
    const added = flow();
    await typeAddress(user);
    expect(await screen.findByLabelText("Code 4 8 2 9 1 3")).toHaveTextContent("482913");
    expect(screen.getByRole("status")).toHaveTextContent("Waiting for you to approve Platter");
    await waitFor(() => expect(added).toEqual(["f00d0001"]));
    expect(mock.jellyfin.servers[0].url).toBe("http://jellyfin.local:8096");
  });

  it("adds the server once, even when an answer is slower than the next check", async () => {
    const user = userEvent.setup();
    let asked = 0;
    const poll = vi.spyOn(mock, "jellyfinQuickPoll").mockImplementation(() => {
      asked++;
      // Every answer says approved, each taking several poll ticks.
      return new Promise((r) => setTimeout(() => r({ state: "done", server: mock.jellyfin.servers[0] ?? ({ tag: "f00d0001" } as never) }), POLL * 4));
    });
    const added = flow();
    await typeAddress(user);
    await waitFor(() => expect(added).toHaveLength(1));
    // Wait as long as a few more ticks would take: no second add, no new questions.
    const after = asked;
    await new Promise((r) => setTimeout(r, POLL * 10));
    expect(added).toHaveLength(1);
    expect(asked).toBe(after);
    poll.mockRestore();
  });

  it("goes to the password form when Quick Connect is turned off", async () => {
    const user = userEvent.setup();
    mock.jellyfin.failure = "quickConnectOff";
    const added = flow();
    await typeAddress(user);
    expect(await screen.findByText(/Quick Connect is turned off on this server/)).toBeInTheDocument();
    await user.type(screen.getByLabelText("User name on Den"), "me");
    const pw = screen.getByLabelText("Password");
    expect(pw).toHaveAttribute("type", "password");
    await user.type(pw, "wrong");
    await user.click(screen.getByRole("button", { name: "Sign in" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(WRONG_PASSWORD);
    // A failed try keeps what was typed, to fix rather than type again.
    expect(pw).toHaveValue("wrong");
    expect(screen.getByLabelText("User name on Den")).toHaveValue("me");
    await user.clear(pw);
    await user.type(pw, "right");
    await user.click(screen.getByRole("button", { name: "Sign in" }));
    await waitFor(() => expect(added).toHaveLength(1));
    expect(QUICK_CONNECT_OFF).toContain("password");
  });

  it("can use a password instead of the code", async () => {
    const user = userEvent.setup();
    mock.jellyfin.approveAfter = 1000;
    flow();
    await typeAddress(user);
    await user.click(await screen.findByRole("button", { name: "Use a password instead" }));
    expect(screen.getByLabelText("Password")).toBeInTheDocument();
    expect(screen.queryByRole("status")).toBeNull();
  });

  it.each([
    ["wrongUrl", WRONG_URL],
    ["notJellyfin", NOT_JELLYFIN],
    ["oldVersion", "This Jellyfin server is version 10.8.13. Platter needs 10.9 or later; update the server to use it here."],
  ] as const)("says why an address can't be used (%s)", async (failure, text) => {
    const user = userEvent.setup();
    mock.jellyfin.failure = failure;
    flow();
    await typeAddress(user, "http://nas.example:8096");
    expect(await screen.findByRole("alert")).toHaveTextContent(text);
    expect(screen.getByLabelText("Jellyfin server address")).toHaveAttribute("aria-invalid", "true");
  });

  it("starts again when the code runs out", async () => {
    const user = userEvent.setup();
    mock.jellyfin.approveAfter = 1000;
    flow();
    await typeAddress(user);
    await screen.findByLabelText("Code 4 8 2 9 1 3");
    mock.jellyfin.failure = "expired";
    expect(await screen.findByRole("alert")).toHaveTextContent(EXPIRED);
    expect(screen.getByLabelText("Jellyfin server address")).toBeInTheDocument();
  });

  it("offers Plex or Jellyfin, with Plex shown as signed in when it is", async () => {
    const user = userEvent.setup();
    let plex = 0;
    flow({ choose: true, plexSignedIn: true, onPlex: () => plex++ });
    const plexButton = screen.getByRole("button", { name: /Plex/ });
    expect(plexButton).toBeDisabled();
    expect(plexButton).toHaveTextContent("Signed in");
    expect(screen.queryByText(/coming soon/i)).toBeNull();
    expect(screen.getByRole("button", { name: /Navidrome or another Subsonic server/ })).toHaveTextContent("Beta");
    await user.click(screen.getByRole("button", { name: /Jellyfin/ }));
    expect(screen.getByLabelText("Jellyfin server address")).toBeInTheDocument();
    expect(plex).toBe(0);
  });
});

describe("first run", () => {
  it("offers Plex and Jellyfin, and a Jellyfin server alone opens the app", async () => {
    const user = userEvent.setup();
    await mock.authSignOut();
    useApp.setState({ servers: null });
    render(<App />);
    expect(await screen.findByRole("button", { name: "Sign in with Plex" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Use a Jellyfin server" })).toBeNull();
    await user.click(screen.getByRole("button", { name: "Use another server" }));
    await user.click(screen.getByRole("button", { name: "Use a Jellyfin server" }));
    mock.jellyfin.failure = "quickConnectOff";
    await user.type(screen.getByLabelText("Jellyfin server address"), "jellyfin.local:8096");
    await user.click(screen.getByRole("button", { name: "Continue" }));
    await user.type(await screen.findByLabelText("User name on Den"), "me");
    await user.type(screen.getByLabelText("Password"), "right");
    await user.click(screen.getByRole("button", { name: "Sign in" }));
    // No Plex account, and the app is open.
    expect(await screen.findByRole("navigation")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Sign in with Plex" })).toBeNull();
    expect(useApp.getState().auth?.state).toBe("signedOut");
    expect(useApp.getState().servers?.map((s) => s.kind)).toEqual(["jellyfin"]);
    // Signing in to Plex as well, later.
    act(() => useUi.getState().setSignInOpen(true));
    expect(await screen.findByRole("heading", { name: "Sign in with Plex" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Not now" }));
    expect(await screen.findByRole("navigation")).toBeInTheDocument();
    await mock.authStart();
    mock.approvePin();
  });
});

describe("Settings, Music servers", () => {
  beforeEach(async () => {
    await mock.authStart();
    mock.approvePin();
    useApp.setState({ auth: { state: "signedIn", username: "listener", needsServerPick: false }, servers: null });
    mock.jellyfin.failure = "quickConnectOff";
    await mock.jellyfinSignInPassword("jellyfin.local:8096", "me", "right");
    mock.jellyfin.failure = null;
  });

  async function serversRow() {
    render(<SettingsPage />);
    const row = await waitFor(() => {
      const r = document.querySelector('[data-setting="servers"]');
      expect(r).toBeTruthy();
      return r as HTMLElement;
    });
    const list = await within(row).findByRole("list", { name: "Music servers" });
    return { row, list };
  }

  it("lists Plex and Jellyfin, the primary marked, and makes another primary", async () => {
    const user = userEvent.setup();
    const { row, list } = await serversRow();
    expect(within(row).getByText("2 servers")).toBeInTheDocument();
    const items = within(list).getAllByRole("listitem").filter((li) => li.classList.contains("library-source"));
    expect(items[0]).toHaveTextContent("Primary");
    expect(items[1]).toHaveTextContent("Jellyfin, signed in as me");
    expect(items[1]).toHaveTextContent("Online");
    await user.click(within(items[1]).getByRole("button", { name: "Make primary" }));
    await waitFor(async () => expect((await mock.serversSaved()).find((s) => s.primary)?.kind).toBe("jellyfin"));
  });

  it("includes each Jellyfin music library", async () => {
    const user = userEvent.setup();
    const { list } = await serversRow();
    const live = await within(list).findByRole("switch", { name: "Include Live tapes on Den" });
    expect(live).toBeChecked();
    await user.click(live);
    await waitFor(() => expect(mock.jellyfin.libraries.f00d0001.map((l) => l.included)).toEqual([true, false]));
    // All on again is saved as "every library".
    await user.click(live);
    await waitFor(() => expect(mock.jellyfin.libraries.f00d0001.every((l) => l.included)).toBe(true));
  });

  it("asks to sign in again after the token is revoked", async () => {
    const user = userEvent.setup();
    mock.jellyfin.revoke("f00d0001");
    const { list } = await serversRow();
    const jf = within(list).getAllByRole("listitem").find((li) => li.getAttribute("data-server") === "f00d0001")!;
    expect(jf).toHaveTextContent("Sign in again");
    await user.click(within(jf).getByRole("button", { name: "Sign in again" }));
    const dialog = await screen.findByRole("dialog", { name: "Sign in again" });
    expect(within(dialog).getByLabelText("Jellyfin server address")).toHaveValue("http://jellyfin.local:8096");
  });

  it("says when the account can't convert or download songs", async () => {
    mock.resetServers();
    mock.jellyfin.canTranscode = false;
    mock.jellyfin.canDownload = false;
    mock.jellyfin.failure = "quickConnectOff";
    await mock.jellyfinSignInPassword("jellyfin.local:8096", "me", "right");
    const { list } = await serversRow();
    await waitFor(() => expect(list).toHaveTextContent("This account can't have songs converted"));
    expect(list).toHaveTextContent("This account can't download songs.");
  });

  it("removes a server after asking", async () => {
    const user = userEvent.setup();
    const { list } = await serversRow();
    const jf = within(list).getAllByRole("listitem").find((li) => li.getAttribute("data-server") === "f00d0001")!;
    await user.click(within(jf).getByRole("button", { name: "Remove" }));
    const dialog = await screen.findByRole("dialog", { name: "Remove Den" });
    await user.click(within(dialog).getByRole("button", { name: "Remove" }));
    await waitFor(() => expect(mock.jellyfin.servers).toHaveLength(0));
    // Plex can't be removed here (it signs out in Account).
    const plex = within(list).getAllByRole("listitem").find((li) => li.getAttribute("data-server") === "0a1b2c3d")!;
    expect(within(plex).queryByRole("button", { name: "Remove" })).toBeNull();
  });

  it("adds a server from Settings", async () => {
    const user = userEvent.setup();
    mock.resetServers();
    mock.jellyfin.approveAfter = 0;
    await serversRow();
    await user.click(screen.getByRole("button", { name: "Add a server" }));
    const dialog = await screen.findByRole("dialog", { name: "Add a server" });
    await user.click(within(dialog).getByRole("button", { name: /Jellyfin/ }));
    await user.type(within(dialog).getByLabelText("Jellyfin server address"), "jellyfin.local:8096");
    await user.click(within(dialog).getByRole("button", { name: "Continue" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Add a server" })).toBeNull(), { timeout: 5000 });
    expect(mock.jellyfin.servers).toHaveLength(1);
  });
});
