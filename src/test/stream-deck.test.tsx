import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS } from "../lib/types";
import { DeckPairing, streamDeckMatches } from "../pages/StreamDeckSettings";
import { SettingsPage } from "../pages/SettingsPage";
import { DeckActions } from "../shell/DeckActions";
import { loadSettings, useSettings } from "../stores/settings";
import { NO_SESSION, useSocial } from "../stores/social";
import { useToasts } from "../stores/toasts";

const mock = backend as MockBackend;

beforeEach(async () => {
  await backend.settingsSet({ ...DEFAULT_SETTINGS });
  await loadSettings();
  useToasts.setState({ toasts: [] });
  useSocial.setState({ listen: NO_SESSION, inviteOpen: false });
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
});
