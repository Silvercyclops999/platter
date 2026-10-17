/**
 * Settings > Integrations > Lidarr: managed and protected artists (switching
 * to Managed asks twice), fixes that show the exact change and ask first,
 * albums waiting in staging (preview, add, undo with Plex's trash), the
 * changes log with Put it back, and practice mode.
 */
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS } from "../lib/types";
import { SettingsPage } from "../pages/SettingsPage";
import { useApp } from "../stores/app";
import { loadSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";

const mock = backend as MockBackend;
const row = (key: string) => document.querySelector<HTMLElement>(`[data-setting="${key}"]`);

beforeEach(async () => {
  mock.lidarr.reset();
  mock.lidarrMock.reset();
  await mock.settingsSet({ ...DEFAULT_SETTINGS, lidarrUrl: "http://192.168.x.x:8686" });
  useSettings.setState({ settings: null });
  await loadSettings();
  await mock.lidarrKeySet("abc123");
  useApp.setState({ auth: { state: "signedIn", username: "listener", needsServerPick: false } });
  useToasts.setState({ toasts: [] });
});

describe("Managed and protected artists", () => {
  it("lists each artist with its mode and why, and Managed asks twice", async () => {
    render(<SettingsPage />);
    await screen.findByText("Protected: 403 files tagged unreleased or leaked");
    expect(screen.getByText("Managed: all files official, no curated tags")).toBeTruthy();
    await userEvent.selectOptions(screen.getByRole("combobox", { name: "Mode for Quiet Hours" }), "managed");
    await screen.findByText("Let Lidarr manage Quiet Hours?");
    await userEvent.click(screen.getByRole("button", { name: "Continue" }));
    await screen.findByText("Are you sure?");
    expect(mock.lidarrMock.prefs).toEqual({});
    await userEvent.click(screen.getByRole("button", { name: "Switch to Managed" }));
    await screen.findByText(/Managed: you chose Managed \(Auto would protect it/);
    expect(mock.lidarrMock.prefs["b0000000-0000-0000-0000-000000000002"].mode).toBe("managed");
  });

  it("backing out of the first question changes nothing", async () => {
    render(<SettingsPage />);
    await screen.findByText("Protected: 403 files tagged unreleased or leaked");
    await userEvent.selectOptions(screen.getByRole("combobox", { name: "Mode for Quiet Hours" }), "managed");
    await userEvent.click(await screen.findByRole("button", { name: "Keep it as it is" }));
    expect(mock.lidarrMock.prefs).toEqual({});
  });

  it("a fix shows the exact change, waits for yes, and lands in the log with Put it back", async () => {
    render(<SettingsPage />);
    await userEvent.click(await screen.findByRole("button", { name: "Check setup" }));
    await userEvent.click((await screen.findAllByRole("button", { name: "Fix this" }))[0]);
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("Turn off Rename Tracks")).toBeTruthy();
    expect(within(dialog).getByText("On")).toBeTruthy();
    expect(within(dialog).getByText("Off")).toBeTruthy();
    expect(mock.lidarrMock.changes).toHaveLength(0);
    await userEvent.click(within(dialog).getByRole("button", { name: "Make this change" }));
    await waitFor(() => expect(mock.lidarrMock.changes).toHaveLength(1));
    const changes = await waitFor(() => {
      const r = row("lidarrChanges");
      expect(r).not.toBeNull();
      return r!;
    });
    await within(changes).findByText("Turn off Rename Tracks");
    await userEvent.click(within(changes).getByRole("button", { name: "Put it back" }));
    await userEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Put it back" }));
    await waitFor(() => expect(mock.lidarrMock.changes[0].putBackAt).not.toBeNull());
  });

  it("turning off release switching for an album asks first, says how to undo it, and is logged without Put it back", async () => {
    render(<SettingsPage />);
    await userEvent.click(await screen.findByRole("button", { name: "Check setup" }));
    await screen.findByText("Lidarr may switch First Light to another edition");
    await userEvent.click((await screen.findAllByRole("button", { name: "Fix this" }))[1]);
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("Turn off Automatically Switch Release for First Light")).toBeTruthy();
    expect(within(dialog).getByText(/change it back in Lidarr/)).toBeTruthy();
    expect(mock.lidarrMock.changes).toHaveLength(0);
    await userEvent.click(within(dialog).getByRole("button", { name: "Make this change" }));
    await waitFor(() => expect(mock.lidarrMock.changes).toHaveLength(1));
    const changes = await waitFor(() => {
      const r = row("lidarrChanges");
      expect(r).not.toBeNull();
      return r!;
    });
    await within(changes).findByText("Turn off Automatically Switch Release for First Light");
    expect(within(changes).queryByRole("button", { name: "Put it back" })).toBeNull();
  });

  it("an edition switch Lidarr made on its own shows as Lidarr's, with nothing to put back", async () => {
    mock.lidarrMock.changes.push({
      id: "seen1",
      at: 1_800_000_000,
      device: "This computer",
      what: "Lidarr switched Panes to the 19-track edition",
      before: "Panes, 13 tracks",
      after: "Panes, 19 tracks. Platter didn't change anything.",
      asked: false,
      undo: null,
      putBackAt: null,
      practice: false,
      byLidarr: true,
    });
    render(<SettingsPage />);
    const changes = await waitFor(() => {
      const r = row("lidarrChanges");
      expect(r).not.toBeNull();
      return r!;
    });
    await within(changes).findByText("Lidarr switched Panes to the 19-track edition");
    expect(within(changes).getByText(/made by Lidarr on its own/)).toBeTruthy();
    expect(within(changes).queryByText(/part of a request you made/)).toBeNull();
    expect(within(changes).queryByRole("button", { name: "Put it back" })).toBeNull();
  });

  it("a waiting album shows its names and tags, is added on yes, and Undo asks about Plex's trash", async () => {
    render(<SettingsPage />);
    const waiting = await waitFor(() => {
      const r = row("lidarrWaiting");
      expect(r).not.toBeNull();
      return r!;
    });
    await within(waiting).findByText(/never adds a partial album/);
    await userEvent.click(within(waiting).getByRole("button", { name: "Show names and tags" }));
    expect(within(waiting).getByText("1-01 - Arrive.flac")).toBeTruthy();
    expect(within(waiting).getAllByText(/VERSION=Official release \(Lidarr, FLAC/).length).toBeGreaterThan(0);
    await userEvent.click(within(waiting).getByRole("checkbox", { name: /Do this automatically for Quiet Hours/ }));
    await userEvent.click(within(waiting).getByRole("button", { name: "Add to library" }));
    await within(waiting).findByText(/In your library:/);
    expect(mock.lidarrMock.prefs["b0000000-0000-0000-0000-000000000002"].autoAdd).toBe(true);
    await userEvent.click(within(waiting).getByRole("button", { name: "Undo" }));
    await screen.findByText("Keep Plex's trash for the move?");
    await userEvent.click(screen.getByRole("button", { name: "Turn it off for the move" }));
    await waitFor(() => expect(mock.lidarrMock.staged[0].state).toBe("undone"));
  });

  it("practice mode says what it is and switches off with one button", async () => {
    render(<SettingsPage />);
    await screen.findByText(/Practice mode until/);
    await userEvent.click(screen.getByRole("button", { name: "Looks right, start for real" }));
    await waitFor(() => expect(useSettings.getState().settings?.lidarrPracticeUntil).toBe(0));
    await screen.findByRole("button", { name: "Practice for a week" });
  });
});
