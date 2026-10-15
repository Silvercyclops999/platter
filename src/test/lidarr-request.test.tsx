/**
 * Lidarr requests in the UI: missing official releases dimmed at the end of
 * an artist's albums (only when Lidarr is set up and its check is green,
 * and the setting is on), the request dialog with where files will land,
 * the first request's typed confirmation, waiting requests, a red check,
 * and the Requests list in Settings.
 */
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS } from "../lib/types";
import { ArtistPage } from "../pages/ArtistPage";
import { missingFor } from "../pages/LidarrRequest";
import { SettingsPage } from "../pages/SettingsPage";
import { useApp } from "../stores/app";
import { loadSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";

const mock = backend as MockBackend;
const artist = () => mock.fx.artists[0];

async function setUp(demo: "ready" | "offline" | "demo", patch: Record<string, unknown> = {}) {
  mock.lidarr.reset(demo);
  await mock.settingsSet({ ...DEFAULT_SETTINGS, lidarrUrl: "http://192.168.x.x:8686", ...patch });
  useSettings.setState({ settings: null });
  await loadSettings();
  if (demo === "demo") await mock.lidarrKeySet("abc123");
}

beforeEach(() => {
  useApp.setState({ auth: { state: "signedIn", username: "listener", needsServerPick: false } });
  useToasts.setState({ toasts: [] });
});

describe("missing official releases on the artist page", () => {
  it("shows albums dimmed at the end of the discography with Request", async () => {
    await setUp("ready");
    render(<ArtistPage id={artist().id} />);
    const paper = await screen.findByLabelText("Paper Lanterns, not in your library");
    expect(paper.className).toContain("missing-release");
    expect(within(paper).getByText("2014 · Album · Not in your library")).toBeInTheDocument();
    expect(within(paper).getByRole("button", { name: "Request Paper Lanterns" })).toBeInTheDocument();
    // At the end of the shelf, after the library's own albums.
    const items = screen.getAllByRole("listitem").filter((li) => li.closest(".discography"));
    expect(items[items.length - 1].querySelector(".missing-release")).not.toBeNull();
    // EPs only under Singles and EPs.
    expect(screen.queryByLabelText("Low Tide EP, not in your library")).toBeNull();
  });

  it("stays hidden with the setting off, without Lidarr, or before a green check", async () => {
    await setUp("ready", { showMissingReleases: false });
    const { unmount } = render(<ArtistPage id={artist().id} />);
    await screen.findByRole("heading", { level: 1, name: artist().title });
    await new Promise((r) => setTimeout(r, 50));
    expect(screen.queryByText(/Not in your library/)).toBeNull();
    unmount();

    await setUp("ready", { lidarrUrl: "" });
    const second = render(<ArtistPage id={artist().id} />);
    await screen.findByRole("heading", { level: 1, name: artist().title });
    await new Promise((r) => setTimeout(r, 50));
    expect(screen.queryByText(/Not in your library/)).toBeNull();
    second.unmount();

    // Set up, but the check hasn't passed (it has a red item).
    await setUp("demo");
    render(<ArtistPage id={artist().id} />);
    await screen.findByRole("heading", { level: 1, name: artist().title });
    await new Promise((r) => setTimeout(r, 50));
    expect(screen.queryByText(/Not in your library/)).toBeNull();
  });

  it("filters by the discography chip", () => {
    const items = [
      { releaseGroupId: "a", title: "A", kind: "album", year: 1, request: null },
      { releaseGroupId: "e", title: "E", kind: "ep", year: 1, request: null },
    ];
    expect(missingFor(items, "popular").map((i) => i.title)).toEqual(["A"]);
    expect(missingFor(items, "albums").map((i) => i.title)).toEqual(["A"]);
    expect(missingFor(items, "singlesEps").map((i) => i.title)).toEqual(["E"]);
    expect(missingFor(items, "live")).toEqual([]);
  });
});

describe("the request dialog", () => {
  it("shows where files land and needs the album's name typed the first time", async () => {
    const user = userEvent.setup();
    await setUp("ready");
    render(<ArtistPage id={artist().id} />);
    await user.click(await screen.findByRole("button", { name: "Request Paper Lanterns" }));
    const dialog = await screen.findByRole("dialog", { name: "Request Paper Lanterns" });
    await within(dialog).findByText(`/downloads/lidarr/${artist().title}`);
    expect(within(dialog).getByText(/with only this album to download/)).toBeInTheDocument();
    const send = within(dialog).getByRole("button", { name: "Request" });
    expect(send).toBeDisabled();
    const field = within(dialog).getByLabelText("This is your first request. Type the album's name to confirm.");
    await user.type(field, "paper");
    expect(send).toBeDisabled();
    await user.type(field, " lanterns");
    expect(send).toBeEnabled();
    await user.click(send);
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    const card = screen.getByLabelText("Paper Lanterns, not in your library");
    await within(card).findByText("Searching");
    expect(mock.lidarr.requests[0].state).toBe("searching");

    // The next request doesn't ask again.
    await user.click(await screen.findByRole("button", { name: "Request Northern Rooms" }));
    const next = await screen.findByRole("dialog", { name: "Request Northern Rooms" });
    await within(next).findByText(`/downloads/lidarr/${artist().title}`);
    expect(within(next).queryByRole("textbox")).toBeNull();
    expect(within(next).getByRole("button", { name: "Request" })).toBeEnabled();
  });

  it("keeps a request waiting when Lidarr can't be reached", async () => {
    const user = userEvent.setup();
    await setUp("offline");
    mock.lidarr.setTypedOnce(true);
    render(<ArtistPage id={artist().id} />);
    await user.click(await screen.findByRole("button", { name: "Request Paper Lanterns" }));
    const dialog = await screen.findByRole("dialog", { name: "Request Paper Lanterns" });
    await within(dialog).findByText(/Lidarr can't be reached right now/);
    expect(within(dialog).getByText(/at the last check/)).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Save request" }));
    const card = screen.getByLabelText("Paper Lanterns, not in your library");
    await within(card).findByText("Waiting to send");
    expect(useToasts.getState().toasts.some((t) => t.message.includes("sends when Platter can reach Lidarr"))).toBe(true);
  });

  it("explains a red check and offers nothing to send", async () => {
    const user = userEvent.setup();
    await setUp("ready");
    render(<ArtistPage id={artist().id} />);
    const button = await screen.findByRole("button", { name: "Request Paper Lanterns" });
    mock.lidarr.setDemo("demo");
    await user.click(button);
    const dialog = await screen.findByRole("dialog", { name: "Request Paper Lanterns" });
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("red items");
    expect(within(dialog).queryByRole("button", { name: "Request" })).toBeNull();
    expect(mock.lidarr.requests).toEqual([]);
  });
});

describe("Settings > Lidarr requests", () => {
  it("lists requests with their state and cancels waiting ones", async () => {
    const user = userEvent.setup();
    await setUp("offline");
    mock.lidarr.setTypedOnce(true);
    await mock.lidarrRequest({ releaseGroupId: "rg-1", title: "Paper Lanterns", artist: "Northlight" });
    render(<SettingsPage />);
    const list = await screen.findByRole("list", { name: "Lidarr requests" });
    expect(within(list).getByText("Waiting to send")).toBeInTheDocument();
    expect(within(list).getByText("Sends when Platter can reach Lidarr.")).toBeInTheDocument();
    expect(document.querySelector('[data-setting="showMissingReleases"]')).not.toBeNull();
    await user.click(within(list).getByRole("button", { name: "Cancel Paper Lanterns" }));
    await screen.findByText(/No requests yet/);
  });
});
