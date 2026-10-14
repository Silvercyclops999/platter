import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import { SettingsPage } from "../pages/SettingsPage";
import { MainView } from "../shell/MainView";
import { PlayerBar } from "../shell/PlayerBar";
import { useShortcuts } from "../shell/useShortcuts";
import { useRouter } from "../stores/router";
import { loadSettings, updateSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";

const route = () => useRouter.getState().entries[useRouter.getState().index].route;

function Host() {
  useShortcuts();
  return <MainView />;
}

beforeEach(async () => {
  await backend.settingsSet({ privateSession: false, reportToPlex: true });
  useSettings.setState({ settings: null });
  await loadSettings();
});
afterEach(() => vi.restoreAllMocks());

describe("settings", () => {
  it("toggles persist through the core", async () => {
    const user = userEvent.setup();
    const set = vi.spyOn(backend, "settingsSet");
    render(<SettingsPage />);
    const priv = screen.getByRole("switch", { name: "Private session" });
    const report = screen.getByRole("switch", { name: "Report plays to Plex" });
    expect(priv).toHaveAttribute("aria-checked", "false");
    expect(report).toHaveAttribute("aria-checked", "true");

    await user.click(priv);
    expect(priv).toHaveAttribute("aria-checked", "true");
    expect(set).toHaveBeenCalledWith({ privateSession: true });
    await user.click(report);
    expect(set).toHaveBeenCalledWith({ reportToPlex: false });

    // A fresh load reads the stored values back.
    useSettings.setState({ settings: null });
    await loadSettings();
    expect(useSettings.getState().settings).toMatchObject({ privateSession: true, reportToPlex: false });
  });

  it("shuffle style switch maps to spread and standard", async () => {
    const user = userEvent.setup();
    const set = vi.spyOn(backend, "settingsSet");
    render(<SettingsPage />);
    const spread = screen.getByRole("switch", { name: "Spread out shuffle" });
    expect(spread).toHaveAttribute("aria-checked", "true");
    await user.click(spread);
    expect(set).toHaveBeenCalledWith({ shuffleStyle: "standard" });
    await user.click(spread);
    expect(set).toHaveBeenCalledWith({ shuffleStyle: "spread" });
  });

  it("shows skeletons until settings arrive", () => {
    useSettings.setState({ settings: null });
    render(<SettingsPage />);
    expect(screen.queryByRole("switch")).not.toBeInTheDocument();
    expect(document.querySelector(".skeleton")).not.toBeNull();
  });

  it("rolls back and explains when saving fails", async () => {
    vi.spyOn(backend, "settingsSet").mockRejectedValue({ kind: "io", message: "disk full" });
    await updateSettings({ privateSession: true });
    expect(useSettings.getState().settings?.privateSession).toBe(false);
    expect(useToasts.getState().toasts.some((t) => t.message.includes("disk full"))).toBe(true);
  });

  it("the player bar shows a private session indicator only when on", async () => {
    render(<PlayerBar />);
    expect(screen.queryByRole("button", { name: "Private session is on" })).not.toBeInTheDocument();
    await updateSettings({ privateSession: true });
    const pill = await screen.findByRole("button", { name: "Private session is on" });
    fireEvent.click(pill);
    expect(route()).toEqual({ name: "settings" });
  });

  it("Ctrl+, opens settings from anywhere, and the header button does too", async () => {
    useRouter.getState().reset({ name: "home" });
    render(<Host />);
    fireEvent.keyDown(window, { key: ",", ctrlKey: true });
    expect(route()).toEqual({ name: "settings" });
    expect(await screen.findByRole("heading", { name: "Settings", level: 1 })).toBeInTheDocument();

    useRouter.getState().reset({ name: "home" });
    await waitFor(() => expect(screen.queryByRole("heading", { name: "Settings", level: 1 })).not.toBeInTheDocument());
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    expect(route()).toEqual({ name: "settings" });
  });
});
