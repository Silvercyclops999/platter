/**
 * Settings > Library > Stats: play history from before Platter and other
 * apps. The Plex row with how its import stands, Tautulli's rows once its
 * address and key are saved in Integrations, Last.fm's once it's connected,
 * removing one source's plays, and a failed update said plainly with a way
 * to try again.
 */
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS } from "../lib/types";
import { historyNote } from "../pages/PlayHistorySettings";
import { SettingsPage } from "../pages/SettingsPage";
import { useApp } from "../stores/app";
import { loadSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";

const mock = backend as MockBackend;

beforeEach(async () => {
  mock.playHistory.reset();
  mock.scrobble.reset();
  await mock.settingsSet({ ...DEFAULT_SETTINGS });
  useSettings.setState({ settings: null });
  await loadSettings();
  useApp.setState({ auth: { state: "signedIn", username: "listener", needsServerPick: false } });
  useToasts.setState({ toasts: [] });
});

const row = (key: string) => document.querySelector<HTMLElement>(`[data-setting="${key}"]`);

describe("play history settings", () => {
  it("shows the Plex row on, with what came in, and keeps Tautulli and Last.fm hidden until set up", async () => {
    render(<SettingsPage />);
    const plex = await screen.findByRole("switch", { name: "Include my Plex listening history" });
    expect(plex.getAttribute("aria-checked")).toBe("true");
    await waitFor(() => expect(row("includePlexHistory")!.textContent).toContain("12,480 plays brought in"));
    expect(row("includePlexHistory")!.textContent).toContain("312 of them songs not in your library");
    expect(row("includePlexHistory")!.textContent).toMatch(/Updated /);
    expect(row("remove-plexHistory")).not.toBeNull();
    expect(row("includeTautulliHistory")).toBeNull();
    expect(row("includeLastfmHistory")).toBeNull();
    // Tautulli's own fields are always in Integrations.
    expect(row("tautulliUrl")).not.toBeNull();
    expect(row("tautulliKey")).not.toBeNull();
  });

  it("turns a source off as a synced setting", async () => {
    render(<SettingsPage />);
    const plex = await screen.findByRole("switch", { name: "Include my Plex listening history" });
    await userEvent.click(plex);
    await waitFor(() => expect(useSettings.getState().settings?.includePlexHistory).toBe(false));
    expect((await mock.settingsGet()).includePlexHistory).toBe(false);
    await waitFor(() => expect(row("includePlexHistory")!.textContent).toContain("Not counted while this is off."));
  });

  it("adds Tautulli's rows once its address and key are saved, and keeps the key out of settings", async () => {
    render(<SettingsPage />);
    const url = await screen.findByRole("textbox", { name: "Tautulli address" });
    await userEvent.type(url, "http://192.168.x.x:8181/{Enter}");
    await waitFor(() => expect(useSettings.getState().settings?.tautulliUrl).toBe("http://192.168.x.x:8181"));
    expect(row("includeTautulliHistory")).toBeNull();
    await userEvent.type(screen.getByLabelText("Tautulli API key"), "abc123");
    await userEvent.click(within(row("tautulliKey")!).getByRole("button", { name: "Save key" }));
    const t = await screen.findByRole("switch", { name: "Include Tautulli history" });
    expect(t.getAttribute("aria-checked")).toBe("true");
    expect(within(row("tautulliKey")!).getByRole("button", { name: "Remove key" })).toBeTruthy();
    expect(JSON.stringify(await mock.settingsGet())).not.toContain("abc123");
    // The next update brings it in, short plays left out.
    await mock.playHistoryUpdate();
    await waitFor(() => expect(row("includeTautulliHistory")!.textContent).toContain("4,966 plays brought in"));
    expect(row("includeTautulliHistory")!.textContent).toContain("311 plays too short to count left out");
  });

  it("adds the Last.fm row once Last.fm is connected for scrobbling", async () => {
    mock.scrobble.setSecret("s");
    mock.scrobble.start();
    mock.scrobble.finish();
    render(<SettingsPage />);
    const lf = await screen.findByRole("switch", { name: "Include Last.fm scrobbles" });
    expect(lf.getAttribute("aria-checked")).toBe("true");
    expect(row("includeLastfmHistory")!.textContent).toContain("Brought in automatically");
  });

  it("removes only one source's plays after asking, and turns it off", async () => {
    render(<SettingsPage />);
    const remove = await waitFor(() => within(row("remove-plexHistory")!).getByRole("button", { name: "Remove" }));
    await userEvent.click(remove);
    const dialog = await screen.findByRole("dialog", { name: "Remove your plays from Plex?" });
    expect(dialog.textContent).toContain("Platter's own plays stay");
    await userEvent.click(within(dialog).getByRole("button", { name: "Remove plays" }));
    await waitFor(() => expect(useSettings.getState().settings?.includePlexHistory).toBe(false));
    await waitFor(() => expect(row("includePlexHistory")!.textContent).toContain("Plays from Plex removed"));
    expect(row("remove-plexHistory")).toBeNull();
    const st = await mock.playHistoryStatus();
    expect(st.sources.find((s) => s.source === "plexHistory")!.plays).toBe(0);
  });

  it("says plainly when an update failed and tries again on request", async () => {
    mock.playHistory.fail("plexHistory", "Your Plex history isn't available right now. Platter will try again later.");
    render(<SettingsPage />);
    await waitFor(() => expect(row("includePlexHistory")!.textContent).toContain("isn't available right now"));
    mock.playHistory.fail("plexHistory", null);
    await userEvent.click(within(row("remove-plexHistory")!).getByRole("button", { name: "Try again" }));
    await waitFor(() => expect(row("includePlexHistory")!.textContent).not.toContain("isn't available"));
    expect(within(row("remove-plexHistory")!).queryByRole("button", { name: "Try again" })).toBeNull();
  });
});

describe("history notes", () => {
  const base = {
    source: "plexHistory" as const,
    setUp: true,
    included: true,
    plays: 0,
    notInLibrary: 0,
    shortPlays: 0,
    updatedAt: null,
    firstReadDone: false,
    importing: false,
    done: 0,
    total: null,
    error: null,
    removed: false,
    paused: false,
  };

  it("reads the first import, a later update and a missing server", () => {
    expect(historyNote({ ...base, importing: true, done: 400, total: 12_000 })).toBe("Bringing in your history from Plex: 400 of 12,000 plays read so far.");
    expect(historyNote({ ...base, importing: true, firstReadDone: true, done: 3 })).toBe("Looking for new plays: 3 plays read.");
    expect(historyNote({ ...base, setUp: false })).toBe("Available when you're signed in to a Plex server.");
    expect(historyNote({ ...base, source: "lastfm", setUp: false })).toBeUndefined();
    expect(historyNote({ ...base, plays: 1, error: "Couldn't reach Tautulli. Platter will try again later.", source: "tautulli" })).toBe(
      "1 play brought in. Couldn't reach Tautulli. Platter will try again later.",
    );
    expect(historyNote(undefined)).toBeUndefined();
    expect(historyNote({ ...base, paused: true, plays: 400 })).toBe("Stopped after 400 plays. Go on brings in the rest.");
  });

  it("never uses an em dash", () => {
    const notes = [
      historyNote({ ...base, importing: true, done: 1 }),
      historyNote({ ...base, plays: 5, notInLibrary: 2, shortPlays: 1, updatedAt: 1 }),
      historyNote({ ...base, removed: true }),
    ];
    for (const n of notes) expect(n).not.toContain("\u2014");
  });
});
