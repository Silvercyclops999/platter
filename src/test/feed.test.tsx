import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { backend } from "../lib/backend";
import { DEFAULT_SETTINGS } from "../lib/types";
import { FeedNote } from "../pages/FeedSettings";
import { SettingsPage } from "../pages/SettingsPage";
import { loadSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";

beforeEach(async () => {
  await backend.settingsSet({ ...DEFAULT_SETTINGS });
  await loadSettings();
  useToasts.setState({ toasts: [] });
});

function row(key: string): HTMLElement {
  const el = document.querySelector<HTMLElement>(`[data-setting="${key}"]`);
  if (!el) throw new Error(`no row ${key}`);
  return el;
}

/** A row that shows once something else is turned on. */
const waitForRow = (key: string) => waitFor(() => row(key));

describe("now-playing feed settings", () => {
  it("is off by default; turning it on shows where it serves", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    const feed = row("nowPlayingFeed");
    expect(feed).toHaveTextContent("Only this computer can reach it");
    const sw = within(feed).getByRole("switch");
    expect(sw).not.toBeChecked();
    expect(await backend.feedStatus()).toEqual({ state: "off" });
    await user.click(sw);
    expect(useSettings.getState().settings?.nowPlayingFeed).toBe(true);
    expect(await within(row("nowPlayingFeed")).findByText("http://127.0.0.1:32540/now-playing.json")).toBeInTheDocument();
    expect(within(row("nowPlayingFeed")).getByText("http://127.0.0.1:32540/now-playing.html")).toBeInTheDocument();
  });

  it("the port waits until the feed is on, saves on leaving the field, and a bad one snaps back", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    expect(document.querySelector('[data-setting="nowPlayingFeedPort"]')).toBeNull();
    await user.click(within(row("nowPlayingFeed")).getByRole("switch"));
    const field = within(await waitForRow("nowPlayingFeedPort")).getByRole("spinbutton");
    expect(field).toHaveValue(32540);
    await user.clear(field);
    await user.type(field, "80");
    fireEvent.blur(field);
    expect(field).toHaveValue(32540);
    expect(useSettings.getState().settings?.nowPlayingFeedPort).toBe(32540);
    await user.clear(field);
    await user.type(field, "40123{Enter}");
    expect(useSettings.getState().settings?.nowPlayingFeedPort).toBe(40123);
    await expect(backend.settingsSet({ nowPlayingFeedPort: 22 })).rejects.toMatchObject({ kind: "invalid" });
  });

  it("says when the port is taken or couldn't open", () => {
    const { rerender } = render(<FeedNote status={{ state: "portInUse", port: 32540 }} />);
    expect(screen.getByRole("alert")).toHaveTextContent("Port 32540 is in use by another program. Pick another port below.");
    rerender(<FeedNote status={{ state: "failed", port: 40000, message: "access denied" }} />);
    expect(screen.getByRole("alert")).toHaveTextContent("Couldn't open port 40000: access denied. Another app may be using it. Choose another port.");
    rerender(<FeedNote status={{ state: "off" }} />);
    expect(screen.queryByRole("alert")).toBeNull();
  });
});
