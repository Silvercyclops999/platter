/**
 * Settings > Integrations > Lidarr: only the address and key fields until
 * both are set, then the read-only setup check with its red, amber and OK
 * findings, the empty root folder confirmation, the away address with its
 * proxy headers, and the failure states.
 */
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
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

beforeEach(async () => {
  mock.lidarr.reset();
  await mock.settingsSet({ ...DEFAULT_SETTINGS });
  useSettings.setState({ settings: null });
  await loadSettings();
  useApp.setState({ auth: { state: "signedIn", username: "listener", needsServerPick: false } });
  useToasts.setState({ toasts: [] });
});

const row = (key: string) => document.querySelector<HTMLElement>(`[data-setting="${key}"]`);

async function setUp(home = "http://192.168.x.x:8686", away = "") {
  await mock.settingsSet({ lidarrUrl: home, lidarrAwayUrl: away });
  await loadSettings();
  await mock.lidarrKeySet("abc123");
}

describe("Lidarr settings", () => {
  it("shows only the address and key fields until both are set", async () => {
    render(<SettingsPage />);
    await screen.findByRole("textbox", { name: "Lidarr home address" });
    expect(row("lidarrAwayUrl")).not.toBeNull();
    expect(row("lidarrKey")).not.toBeNull();
    expect(row("lidarrCheck")).toBeNull();
    expect(row("lidarrHeaders")).toBeNull();
    expect(screen.queryByRole("button", { name: /request/i })).toBeNull();

    const url = screen.getByRole("textbox", { name: "Lidarr home address" });
    await userEvent.type(url, "http://192.168.x.x:8686/{Enter}");
    await waitFor(() => expect(useSettings.getState().settings?.lidarrUrl).toBe("http://192.168.x.x:8686"));
    expect(row("lidarrCheck")).toBeNull();

    await userEvent.type(screen.getByLabelText("Lidarr API key"), "abc123");
    await userEvent.click(within(row("lidarrKey")!).getByRole("button", { name: "Save key" }));
    await screen.findByRole("button", { name: "Check setup" });
    expect(within(row("lidarrKey")!).getByRole("button", { name: "Remove key" })).toBeTruthy();
    // The key never goes into settings.
    expect(JSON.stringify(await mock.settingsGet())).not.toContain("abc123");
  });

  it("lists red first, then amber, then OK, and says nothing will be sent", async () => {
    await setUp();
    render(<SettingsPage />);
    await userEvent.click(await screen.findByRole("button", { name: "Check setup" }));
    await screen.findByText("Managed artists keep working. Platter won't do anything for protected artists until the red items are fixed.");
    const items = within(row("lidarrCheck")!).getAllByRole("listitem");
    const levels = items.map((li) => li.className.match(/lidarr-finding--(\w+)/)?.[1]);
    expect(levels.filter((l) => l === "red")).toHaveLength(1);
    expect(levels.filter((l) => l === "amber")).toHaveLength(2);
    expect(levels.indexOf("amber")).toBeGreaterThan(levels.indexOf("red"));
    expect(levels.lastIndexOf("amber")).toBeLessThan(levels.indexOf("ok"));
    expect(screen.getByText(/a protected artist in your Plex music/)).toBeTruthy();
    expect(screen.getByText(/They don't stop anything on their own/)).toBeTruthy();
    expect(screen.getByText(/answered on its home address/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Check again" })).toBeTruthy();
  });

  it("all clear", async () => {
    mock.lidarr.setDemo("clear");
    await setUp();
    render(<SettingsPage />);
    await userEvent.click(await screen.findByRole("button", { name: "Check setup" }));
    await screen.findByText(/Lidarr can't reach your curated music/);
    expect(document.querySelector(".lidarr-finding--red, .lidarr-finding--amber")).toBeNull();
  });

  it("an empty root folder needs the user to say it's different, and can be undone", async () => {
    mock.lidarr.setDemo("empty");
    await setUp();
    render(<SettingsPage />);
    await userEvent.click(await screen.findByRole("button", { name: "Check setup" }));
    await screen.findByText("Lidarr's folder /lidarr is empty");
    await userEvent.click(screen.getByRole("button", { name: "It's a different folder" }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(/even under another name/)).toBeTruthy();
    await userEvent.click(within(dialog).getByRole("button", { name: "It's a different folder" }));
    await screen.findByText("You said /lidarr is a separate folder");
    expect(screen.getByText(/Lidarr can't reach your curated music/)).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: "Undo" }));
    await screen.findByText("Lidarr's folder /lidarr is empty");
  });

  it.each([
    ["unreachable", "Lidarr isn't reachable"],
    ["badKey", "Lidarr didn't accept this API key"],
    ["old", "Platter needs Lidarr 2.0 or later"],
  ] as const)("shows %s in plain words", async (demo, text) => {
    mock.lidarr.setDemo(demo);
    await setUp();
    render(<SettingsPage />);
    await userEvent.click(await screen.findByRole("button", { name: "Check setup" }));
    expect((await screen.findByRole("alert")).textContent).toContain(text);
  });

  it("away from home: the away address answers, takes proxy headers, and warns only for plain http", async () => {
    mock.lidarr.setDemo("away");
    await setUp("http://192.168.x.x:8686", "https://lidarr.example.com");
    render(<SettingsPage />);
    const field = await screen.findByLabelText("Extra headers for the away address");
    fireEvent.change(field, { target: { value: "CF-Access-Client-Id: id\nCF-Access-Client-Secret: s3cret" } });
    await userEvent.click(screen.getByRole("button", { name: "Save headers" }));
    await screen.findByText("Saved: CF-Access-Client-Id, CF-Access-Client-Secret.");
    expect(document.body.textContent).not.toContain("s3cret");
    await userEvent.click(screen.getByRole("button", { name: "Check setup" }));
    await screen.findByText(/answered on its away address/);
    expect(screen.queryByText("Your API key would travel unencrypted")).toBeNull();

    await mock.settingsSet({ lidarrAwayUrl: "http://203.0.113.9:8686" });
    await loadSettings();
    await userEvent.click(await screen.findByRole("button", { name: "Check setup" }));
    await screen.findByText("Your API key would travel unencrypted");
    expect(screen.getByText(/Lidarr can't reach your curated music/)).toBeTruthy(), "a warning, never a block";
  });

  it("refuses a bad address and bad headers", async () => {
    await expect(mock.settingsSet({ lidarrUrl: "nas:8686" })).rejects.toMatchObject({ kind: "invalid" });
    await expect(mock.lidarrHeadersSet("X-Api-Key: nope")).rejects.toMatchObject({ kind: "invalid" });
    await expect(mock.lidarrKeySet("not a key!")).rejects.toMatchObject({ kind: "invalid" });
  });
});
