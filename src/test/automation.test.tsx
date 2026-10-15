import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { Toaster } from "../components/Toast";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS } from "../lib/types";
import { SettingsPage } from "../pages/SettingsPage";
import { automationMatches } from "../pages/StreamDeckSettings";
import { DeckActions } from "../shell/DeckActions";
import { useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";

const mock = backend as MockBackend;

beforeEach(async () => {
  mock.autoPaired = false;
  await backend.settingsSet({ ...DEFAULT_SETTINGS });
  await loadSettings();
  useToasts.setState({ toasts: [] });
  useRouter.getState().reset();
});

function row(): HTMLElement {
  const el = document.querySelector<HTMLElement>('[data-setting="allowAutomation"]');
  if (!el) throw new Error("no Automation row");
  return el;
}

describe("Allow automation", () => {
  it("is off by default; on shows its own pairing code and how to pair", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    const sw = within(row()).getByRole("switch", { name: "Allow automation" });
    expect(sw).not.toBeChecked();
    expect(screen.queryByTestId("automation-pairing")).toBeNull();
    await user.click(sw);
    expect(useSettings.getState().settings?.allowAutomation).toBe(true);
    const pairing = await screen.findByTestId("automation-pairing");
    expect(pairing).toHaveTextContent(/Pairing code: \d{6}/);
    expect(pairing).toHaveTextContent(/platter --pair \d{6}/);
    // The Stream Deck row stays off and shows no code.
    expect(screen.queryByTestId("deck-pairing")).toBeNull();
  });

  it("Revoke shows once paired and gives a new code", async () => {
    mock.autoPaired = true;
    await backend.settingsSet({ allowAutomation: true });
    await loadSettings();
    render(
      <>
        <SettingsPage />
        <Toaster />
      </>,
    );
    const pairing = await screen.findByTestId("automation-pairing");
    const before = (await backend.deckStatus()).automationCode;
    expect(pairing).toHaveTextContent("A script is paired.");
    const btn = await waitFor(() => within(screen.getByTestId("automation-pairing")).getByRole("button", { name: "Revoke" }));
    fireEvent.click(btn);
    await waitFor(() => expect(screen.getByTestId("automation-pairing")).toHaveTextContent(/platter --pair/));
    expect((await backend.deckStatus()).automationCode).not.toBe(before);
    expect(useToasts.getState().toasts.map((t) => t.message)).toContain("Scripts were unpaired. Pair again with the new code.");
  });

  it("is found by search for scripts, Home Assistant and the command line", () => {
    expect(automationMatches("home assistant")).toBe(true);
    expect(automationMatches("command line")).toBe(true);
    expect(automationMatches("raycast")).toBe(true);
    expect(automationMatches("equalizer")).toBe(false);
  });

  it("stays on this computer and defaults off", () => {
    expect(DEFAULT_SETTINGS.allowAutomation).toBe(false);
  });
});

describe("command links in the window", () => {
  it("a link that opens a page or searches goes there", async () => {
    render(<DeckActions />);
    await act(async () => {
      mock.emit("deck://action", { kind: "open", route: { name: "search", query: "night drive" } });
    });
    const r = useRouter.getState();
    expect(r.entries[r.index].route).toEqual({ name: "search", query: "night drive" });
  });

  it("a link that couldn't run says why", async () => {
    render(<DeckActions />);
    await act(async () => {
      mock.emit("deck://action", { kind: "notice", message: "Turn on Allow automation in Settings to control Platter from links." });
    });
    await waitFor(() => expect(useToasts.getState().toasts.map((t) => t.message)).toContain("Turn on Allow automation in Settings to control Platter from links."));
  });
});
