import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS } from "../lib/types";
import { DeckPairing, streamDeckMatches } from "../pages/StreamDeckSettings";
import { SettingsPage } from "../pages/SettingsPage";
import { DeckActions } from "../shell/DeckActions";
import { DeckPairPrompt } from "../shell/DeckPairPrompt";
import { fullWindow, useFullWindow } from "../stores/fullWindow";
import { loadSettings, useSettings } from "../stores/settings";
import { useUi } from "../stores/ui";
import { NO_SESSION, useSocial } from "../stores/social";
import { useToasts } from "../stores/toasts";

const mock = backend as MockBackend;

beforeEach(async () => {
  await backend.settingsSet({ ...DEFAULT_SETTINGS });
  await loadSettings();
  useToasts.setState({ toasts: [] });
  useSocial.setState({ listen: NO_SESSION, inviteOpen: false });
  mock.deckPlugin.status = { state: "install", version: "0.2.0.0" };
  mock.deckPlugin.opened = 0;
  mock.deckPlugin.downloadOpened = 0;
});

function row(): HTMLElement {
  const el = document.querySelector<HTMLElement>('[data-setting="allowStreamDeck"]');
  if (!el) throw new Error("no Stream Deck row");
  return el;
}

describe("Stream Deck settings", () => {
  it("is off by default; turning it on shows the pairing code", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    const sw = within(row()).getByRole("switch");
    expect(sw).not.toBeChecked();
    expect(row()).toHaveTextContent("never your network");
    expect(screen.queryByTestId("deck-pairing")).toBeNull();
    await user.click(sw);
    expect(useSettings.getState().settings?.allowStreamDeck).toBe(true);
    const pairing = await screen.findByTestId("deck-pairing");
    expect(pairing).toHaveTextContent(/Pairing code: \d{6}/);
    expect(within(pairing).queryByRole("button", { name: "Revoke" })).toBeNull();
  });

  it("revoke shows only once paired and calls the core", async () => {
    const user = userEvent.setup();
    const onRevoke = vi.fn();
    const { rerender } = render(<DeckPairing status={{ state: "on", port: 32541, code: "123456", paired: true, automationCode: null, automationPaired: false }} onRevoke={onRevoke} />);
    expect(screen.getByText("A Stream Deck is paired.")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Revoke" }));
    expect(onRevoke).toHaveBeenCalledOnce();
    rerender(<DeckPairing status={{ state: "portInUse", port: 32541, code: null, paired: false, automationCode: null, automationPaired: false }} onRevoke={onRevoke} />);
    expect(screen.getByRole("alert")).toHaveTextContent("Port 32541 is in use by another program");
    rerender(<DeckPairing status={{ state: "failed", port: 32541, message: "denied", code: null, paired: false, automationCode: null, automationPaired: false }} onRevoke={onRevoke} />);
    expect(screen.getByRole("alert")).toHaveTextContent("Couldn't open port 32541: denied");
  });

  it("revoking makes a new code", async () => {
    await backend.settingsSet({ allowStreamDeck: true });
    const before = await backend.deckStatus();
    const after = await backend.deckRevoke();
    expect(after.paired).toBe(false);
    expect(after.code).not.toBe(before.code);
  });

  it("is found by search", () => {
    expect(streamDeckMatches("stream deck")).toBe(true);
    expect(streamDeckMatches("pairing")).toBe(true);
    expect(streamDeckMatches("crossfade")).toBe(false);
  });

  it("the setting stays on this computer and defaults off", () => {
    expect(DEFAULT_SETTINGS.allowStreamDeck).toBe(false);
  });
});

describe("Stream Deck actions in the window", () => {
  it("Listen along starts a session and shows the invite", async () => {
    render(<DeckActions />);
    await act(async () => {
      mock.emit("deck://action", { kind: "listenAlong" });
    });
    await waitFor(() => expect(useSocial.getState().listen.role).toBe("host"));
    expect(useSocial.getState().inviteOpen).toBe(true);
  });

  it("Listen along while a guest only explains", async () => {
    useSocial.setState({ listen: { ...NO_SESSION, role: "guest" } });
    render(<DeckActions />);
    await act(async () => {
      mock.emit("deck://action", { kind: "listenAlong" });
    });
    expect(useToasts.getState().toasts.map((t) => t.message).join(" ")).toContain("listening along with someone");
    expect(useSocial.getState().listen.role).toBe("guest");
  });

  it("Request album opens the request dialog", async () => {
    render(<DeckActions />);
    await act(async () => {
      mock.emit("deck://action", { kind: "requestAlbum", input: { releaseGroupId: "rg-1", title: "Late", artist: "Rue Ellis", artistId: null } });
    });
    expect(await screen.findByRole("dialog")).toBeInTheDocument();
  });

  it("Add to playlist opens a playlist picker", async () => {
    render(<DeckActions />);
    await act(async () => {
      mock.emit("deck://action", { kind: "addToPlaylist", trackIds: ["t1"] });
    });
    const d = await screen.findByRole("dialog", { name: "Add to playlist" });
    expect(within(d).getByRole("searchbox", { name: "Find a playlist" })).toBeInTheDocument();
    await userEvent.setup().keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Add to playlist" })).toBeNull());
  });

  it("Lyrics, now playing and queue open their views, and the window reports the lyrics", async () => {
    await act(async () => {
      await fullWindow.close();
    });
    useUi.getState().setQueueOpen(false);
    render(<DeckActions />);
    await act(async () => {
      mock.emit("deck://action", { kind: "lyrics", open: true });
    });
    expect(useFullWindow.getState()).toMatchObject({ open: true, mode: "lyrics" });
    await waitFor(() => expect(mock.deckUi.lyrics).toBe(true));
    await act(async () => {
      mock.emit("deck://action", { kind: "lyrics", open: false });
    });
    expect(useFullWindow.getState().open).toBe(false);
    await waitFor(() => expect(mock.deckUi.lyrics).toBe(false));
    await act(async () => {
      mock.emit("deck://action", { kind: "queue" });
    });
    expect(useUi.getState().queueOpen).toBe(true);
    await act(async () => {
      mock.emit("deck://action", { kind: "nowPlaying" });
    });
    expect(useFullWindow.getState().open).toBe(true);
    await act(async () => {
      await fullWindow.close();
    });
  });

  it("Listen on opens the device list", async () => {
    useUi.getState().setOutputOpen(false);
    render(<DeckActions />);
    await act(async () => {
      mock.emit("deck://action", { kind: "listenOn" });
    });
    expect(useUi.getState().outputOpen).toBe(true);
  });
});

describe("Install for Stream Deck", () => {
  it("installs: turns on Allow Stream Deck and opens the bundled plugin", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    const row = await screen.findByTestId("deck-plugin");
    expect(useSettings.getState().settings?.allowStreamDeck).toBe(false);
    await user.click(within(row).getByRole("button", { name: "Install for Stream Deck" }));
    await waitFor(() => expect(mock.deckPlugin.opened).toBe(1));
    expect(useSettings.getState().settings?.allowStreamDeck).toBe(true);
    expect(useToasts.getState().toasts.map((t) => t.message).join(" ")).toContain("press any Platter key and choose Allow");
  });

  it("the same version says Installed with Reinstall", async () => {
    mock.deckPlugin.status = { state: "installed", version: "0.2.0.0", installed: "0.2.0.0" };
    const user = userEvent.setup();
    render(<SettingsPage />);
    const row = await screen.findByTestId("deck-plugin");
    expect(row).toHaveTextContent("Installed");
    expect(within(row).queryByRole("button", { name: "Install for Stream Deck" })).toBeNull();
    await user.click(within(row).getByRole("button", { name: "Reinstall" }));
    await waitFor(() => expect(mock.deckPlugin.opened).toBe(1));
  });

  it("an older version shows Update", async () => {
    mock.deckPlugin.status = { state: "update", version: "0.2.0.0", installed: "0.1.0.0" };
    const user = userEvent.setup();
    render(<SettingsPage />);
    const row = await screen.findByTestId("deck-plugin");
    expect(row).toHaveTextContent("Version 0.1.0.0 is installed");
    await user.click(within(row).getByRole("button", { name: "Update" }));
    await waitFor(() => expect(mock.deckPlugin.opened).toBe(1));
  });

  it("without the Stream Deck app: one line with Elgato's download page, nothing else", async () => {
    mock.deckPlugin.status = { state: "noApp" };
    const user = userEvent.setup();
    render(<SettingsPage />);
    const line = await screen.findByTestId("deck-plugin-missing");
    expect(screen.queryByTestId("deck-plugin")).toBeNull();
    expect(within(line).getAllByRole("button")).toHaveLength(1);
    await user.click(within(line).getByRole("button", { name: "Elgato's download page" }));
    await waitFor(() => expect(mock.deckPlugin.downloadOpened).toBe(1));
    expect(mock.deckPlugin.opened).toBe(0);
  });

  it("no em dashes in what it says", async () => {
    for (const status of [
      { state: "install", version: "0.2.0.0" },
      { state: "installed", version: "0.2.0.0", installed: "0.2.0.0" },
      { state: "update", version: "0.2.0.0", installed: "0.1.0.0" },
      { state: "noApp" },
    ] as const) {
      mock.deckPlugin.status = status;
      const { unmount } = render(<SettingsPage />);
      await screen.findByTestId(status.state === "noApp" ? "deck-plugin-missing" : "deck-plugin");
      expect(document.body.textContent).not.toContain("\u2014");
      unmount();
    }
  });
});

describe("Allow Stream Deck to control Platter?", () => {
  it("shows when the Stream Deck asks; Allow pairs", async () => {
    await backend.settingsSet({ allowStreamDeck: true });
    const user = userEvent.setup();
    render(<DeckPairPrompt />);
    expect(screen.queryByRole("dialog")).toBeNull();
    await act(async () => {
      mock.askDeckPairing("req1");
    });
    const dialog = await screen.findByRole("dialog", { name: "Allow Stream Deck to control Platter?" });
    // Don't allow has the focus, so Enter never pairs by accident.
    await waitFor(() => expect(within(dialog).getByRole("button", { name: "Don't allow" })).toHaveFocus());
    await user.click(within(dialog).getByRole("button", { name: "Allow" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    await waitFor(async () => expect((await backend.deckStatus()).paired).toBe(true));
  });

  it("Don't allow and Escape pair nothing", async () => {
    await backend.deckRevoke();
    const user = userEvent.setup();
    render(<DeckPairPrompt />);
    await act(async () => {
      mock.askDeckPairing("req2");
    });
    await user.click(await screen.findByRole("button", { name: "Don't allow" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    await act(async () => {
      mock.askDeckPairing("req3");
    });
    await screen.findByRole("dialog");
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect((await backend.deckStatus()).paired).toBe(false);
  });

  it("a prompt already open when the window loads still shows", async () => {
    mock.askDeckPairing("req4");
    render(<DeckPairPrompt />);
    expect(await screen.findByRole("dialog")).toBeInTheDocument();
    await act(async () => {
      mock.emit("deck://pair-request", { id: null });
    });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });
});
