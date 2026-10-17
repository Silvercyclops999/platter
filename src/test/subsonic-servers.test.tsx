/**
 * Navidrome and other Subsonic servers: adding one with an API key or a
 * password, each way that can fail, signing in again from Settings, the
 * change-check setting, and picking up a queue another app saved. Each
 * failure state is faked through the mock backend.
 */
import { ONLINE_EXTRAS } from "./onlineExtras";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { KEY_REFUSED, NO_API_KEYS, NOT_SUBSONIC, WRONG_PASSWORD, WRONG_URL } from "../lib/mock/subsonic";
import { DEFAULT_SETTINGS } from "../lib/types";
import { AddServerFlow } from "../pages/AddServer";
import { SettingsPage } from "../pages/SettingsPage";
import { ResumeFrom, offerLine } from "../shell/ResumeFrom";
import { useApp } from "../stores/app";
import { ensureCaps, useCaps } from "../stores/caps";
import { useSettings } from "../stores/settings";

const mock = backend as MockBackend;

beforeEach(() => {
  mock.resetServers();
  useSettings.setState({ settings: { ...DEFAULT_SETTINGS, ...ONLINE_EXTRAS } });
});

function flow(props: Partial<Parameters<typeof AddServerFlow>[0]> = {}) {
  const added: string[] = [];
  render(<AddServerFlow kind="subsonic" onDone={(s) => added.push(s.tag)} onCancel={() => {}} {...props} />);
  return added;
}

async function typeAddress(user: ReturnType<typeof userEvent.setup>, url = "navidrome.local:4533") {
  await user.type(screen.getByLabelText("Server address"), url);
  await user.click(screen.getByRole("button", { name: "Continue" }));
}

describe("adding a Navidrome or other Subsonic server", () => {
  it("asks for an API key first, and keeps a refused one to fix", async () => {
    const user = userEvent.setup();
    const added = flow();
    await typeAddress(user);
    const key = await screen.findByLabelText("API key from Navidrome on navidrome.local");
    expect(key).toHaveAttribute("type", "password");
    expect(screen.getByText(/Make an API key in your account settings/)).toBeInTheDocument();
    await user.type(key, "old-key");
    await user.click(screen.getByRole("button", { name: "Sign in" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(KEY_REFUSED);
    expect(key).toHaveValue("old-key");
    await user.clear(key);
    await user.type(key, "good-key");
    await user.click(screen.getByRole("button", { name: "Sign in" }));
    await waitFor(() => expect(added).toEqual(["5ab50001"]));
    expect(mock.subsonic.servers[0]).toMatchObject({ kind: "subsonic", url: "http://navidrome.local:4533", user: "me" });
  });

  it("signs in with a password instead, saying why a key is safer", async () => {
    const user = userEvent.setup();
    const added = flow();
    await typeAddress(user, "http://navidrome.local:4533/app/#/album");
    await user.click(await screen.findByRole("button", { name: "Use a password instead" }));
    expect(screen.getByText(/keeps it in this computer's keychain/)).toHaveTextContent("An API key is safer");
    await user.type(screen.getByLabelText("User name on Navidrome on navidrome.local"), "me");
    const pw = screen.getByLabelText("Password");
    await user.type(pw, "wrong");
    await user.click(screen.getByRole("button", { name: "Sign in" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(WRONG_PASSWORD);
    expect(pw).toHaveValue("wrong");
    await user.clear(pw);
    await user.type(pw, "right");
    await user.click(screen.getByRole("button", { name: "Sign in" }));
    await waitFor(() => expect(added).toHaveLength(1));
    expect(mock.subsonic.servers[0].url).toBe("http://navidrome.local:4533");
  });

  it("goes straight to the password on a server without API keys", async () => {
    const user = userEvent.setup();
    mock.subsonic.failure = "noExtensions";
    flow();
    await typeAddress(user, "music.example:4040");
    expect(await screen.findByText(/doesn't take API keys/)).toBeInTheDocument();
    expect(screen.getByLabelText("Password")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Use an API key instead" })).toBeNull();
    expect(screen.getByText(/keeps it in this computer's keychain/)).not.toHaveTextContent("An API key is safer");
    await expect(mock.subsonicSignIn("music.example:4040", { apiKey: "good-key" })).rejects.toMatchObject({ message: NO_API_KEYS });
  });

  it.each([
    ["wrongUrl", WRONG_URL],
    ["notSubsonic", NOT_SUBSONIC],
  ] as const)("says why an address can't be used (%s)", async (failure, text) => {
    const user = userEvent.setup();
    mock.subsonic.failure = failure;
    flow();
    await typeAddress(user, "http://nas.example:4533");
    expect(await screen.findByRole("alert")).toHaveTextContent(text);
    expect(screen.getByLabelText("Server address")).toHaveAttribute("aria-invalid", "true");
  });

  it("is offered next to Plex and Jellyfin, in beta", async () => {
    const user = userEvent.setup();
    flow({ choose: true });
    const os = screen.getByRole("button", { name: /Navidrome or another Subsonic server/ });
    expect(os).toHaveTextContent("Beta");
    await user.click(os);
    expect(screen.getByLabelText("Server address")).toHaveAttribute("placeholder", "http://navidrome.local:4533");
  });
});

describe("Settings with a Subsonic server", () => {
  beforeEach(async () => {
    useApp.setState({ servers: null });
    await mock.subsonicSignIn("navidrome.local:4533", { apiKey: "good-key" });
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

  it("lists it and signs in again at its own address", async () => {
    const user = userEvent.setup();
    mock.subsonic.revoke("5ab50001");
    const { list } = await serversRow();
    const os = within(list).getAllByRole("listitem").find((li) => li.getAttribute("data-server") === "5ab50001")!;
    expect(os).toHaveTextContent("Subsonic, signed in as me");
    await user.click(within(os).getByRole("button", { name: "Sign in again" }));
    const dialog = await screen.findByRole("dialog", { name: "Sign in again" });
    expect(within(dialog).getByLabelText("Server address")).toHaveValue("http://navidrome.local:4533");
  });

  it("says when the account can't download songs", async () => {
    mock.resetServers();
    mock.subsonic.canDownload = false;
    await mock.subsonicSignIn("navidrome.local:4533", { user: "me", password: "right" });
    const { list } = await serversRow();
    await waitFor(() => expect(list).toHaveTextContent("This account can't download songs."));
  });

  it("removes it after asking", async () => {
    const user = userEvent.setup();
    const { list } = await serversRow();
    const os = within(list).getAllByRole("listitem").find((li) => li.getAttribute("data-server") === "5ab50001")!;
    await user.click(within(os).getByRole("button", { name: "Remove" }));
    const dialog = await screen.findByRole("dialog", { name: /Remove Navidrome/ });
    await user.click(within(dialog).getByRole("button", { name: "Remove" }));
    await waitFor(() => expect(mock.subsonic.servers).toHaveLength(0));
  });

  it("sets how often it's checked for changes, from the times offered", async () => {
    const user = userEvent.setup();
    useApp.setState({ servers: await mock.serversSaved() });
    render(<SettingsPage />);
    const row = await waitFor(() => {
      const r = document.querySelector('[data-setting="subsonicCheckMinutes"]');
      expect(r).toBeTruthy();
      return r as HTMLElement;
    });
    expect(row).toHaveTextContent("Check Subsonic servers for changes every");
    expect(row).toHaveTextContent("This device only.");
    await user.selectOptions(within(row).getByRole("combobox"), "15");
    await waitFor(async () => expect((await mock.settingsGet()).subsonicCheckMinutes).toBe(15));
    await expect(mock.settingsSet({ subsonicCheckMinutes: 7 })).rejects.toMatchObject({ kind: "invalid" });
  });

  it("offers to keep the queue on servers that hold one", async () => {
    const user = userEvent.setup();
    useCaps.setState({ all: await mock.backendCaps() });
    render(<SettingsPage />);
    const row = await waitFor(() => {
      const r = document.querySelector('[data-setting="saveQueueOnServer"]');
      expect(r).toBeTruthy();
      return r as HTMLElement;
    });
    expect(row).toHaveTextContent("Keep my queue on the server");
    const sw = within(row).getByRole("switch");
    expect(sw).toBeChecked();
    await user.click(sw);
    await waitFor(async () => expect((await mock.settingsGet()).saveQueueOnServer).toBe(false));
  });

  it("hides the change check without a Subsonic server", async () => {
    mock.resetServers();
    useApp.setState({ servers: await mock.serversSaved() });
    render(<SettingsPage />);
    await waitFor(() => expect(document.querySelector('[data-setting="servers"]')).toBeTruthy());
    expect(document.querySelector('[data-setting="subsonicCheckMinutes"]')).toBeNull();
  });
});

describe("picking up a queue saved on a server", () => {
  it("shows queues other apps saved and plays one", async () => {
    const user = userEvent.setup();
    const s = await mock.subsonicSignIn("navidrome.local:4533", { apiKey: "good-key" });
    mock.subsonic.queues[s.tag] = {
      tag: s.tag,
      serverName: s.name,
      trackCount: 12,
      currentId: `${s.tag}:s4`,
      positionMs: 30_000,
      changedBy: "Phone app",
      changedAt: Math.floor(Date.now() / 1000) - 300,
    };
    useCaps.setState({ all: await mock.backendCaps() });
    ensureCaps();
    render(<ResumeFrom />);
    const row = await screen.findByRole("button", { name: "Play the queue saved on Navidrome on navidrome.local" });
    expect(row).toHaveTextContent("12 songs, saved by Phone app 5 min ago");
    expect(screen.getByRole("heading", { name: "Pick up where you left off" })).toBeInTheDocument();
    await user.click(row);
    await waitFor(() => expect(mock.subsonic.played).toEqual([s.tag]));
  });

  it("stays hidden when no server keeps a queue", async () => {
    const spy = vi.spyOn(mock, "resumeQueues");
    useCaps.setState({ all: { caps: (await mock.backendCaps()).caps, servers: [] } });
    const { container } = render(<ResumeFrom />);
    expect(container).toBeEmptyDOMElement();
    expect(spy).not.toHaveBeenCalled();
    spy.mockRestore();
  });

  it("says when and by what", () => {
    const base = { tag: "t", serverName: "S", trackCount: 1, currentId: "t:1", positionMs: 0, changedBy: null, changedAt: null };
    expect(offerLine(base)).toBe("1 song");
    expect(offerLine({ ...base, trackCount: 3, changedBy: "Web", changedAt: 1000 }, 1000 + 7200)).toBe("3 songs, saved by Web 2 h ago");
    expect(offerLine({ ...base, changedAt: 1000 }, 1010)).toBe("1 song just now");
  });
});
