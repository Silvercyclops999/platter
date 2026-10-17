/**
 * Listening history imports and "Played a lot, not in your library": the
 * report and toast for a history-only download (never "0 of 0"), a file with
 * nothing usable, the list on the Missing page (songs, albums, filters,
 * hiding, rows leaving when the library gains the song, the empty state),
 * Request and "Request these" through the usual preview and one confirm,
 * and the settings. All data is made up by the mock.
 */
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS } from "../lib/types";
import { importSourceName } from "../lib/importStrings";
import { ImportPage } from "../pages/ImportPage";
import { LibraryImportSettings } from "../pages/LibraryImportSettings";
import { importFrom, resetImportListeners } from "../pages/importShared";
import { BATCH_MAX, PlayedSettings, playTime } from "../pages/PlayedALot";
import { useApp } from "../stores/app";
import { useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";

const mock = backend as MockBackend;
const toasts = () => useToasts.getState().toasts;
const route = () => useRouter.getState().entries[useRouter.getState().index].route;
const HISTORY = "Downloads/my_extended_history.zip";

async function settings(patch: Record<string, unknown> = {}) {
  await mock.settingsSet({ ...DEFAULT_SETTINGS, importSources: ["streaming"], ...patch });
  useSettings.setState({ settings: null });
  await loadSettings();
}

async function withHistory(patch: Record<string, unknown> = {}) {
  await settings(patch);
  mock.importMock.run("streaming", HISTORY);
}

const rows = () => within(screen.getByRole("list", { name: /^(Songs|Albums) played a lot$/ })).getAllByRole("listitem");
const titles = () => rows().map((r) => r.getAttribute("aria-label"));

beforeEach(async () => {
  mock.importMock.reset();
  mock.lidarr.reset("ready");
  resetImportListeners();
  useApp.setState({ auth: { state: "signedIn", username: "listener", needsServerPick: false } });
  useToasts.setState({ toasts: [] });
  useRouter.getState().reset({ name: "home" });
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("history-only imports", () => {
  it("reports plays, years and songs found, with a toast that opens Played a lot", async () => {
    await settings();
    vi.spyOn(mock, "importChoose").mockResolvedValue(HISTORY);
    const r = await importFrom("streaming", false);
    expect(r?.songs.total).toBe(0);
    const t = toasts().find((x) => x.message.startsWith(importSourceName("streaming")))!;
    expect(t.message).toMatch(/^.+: 5,210 plays imported, \d{4} to \d{4}\. 610 of 1,240 songs are in your library\.$/);
    expect(t.message).not.toContain("0 of 0");
    expect(t.action?.label).toBe("See Played a lot");
    act(() => t.action!.run());
    expect(route()).toEqual({ name: "import", view: "played" });
  });

  it("shows the history line and its links in Settings and on the report", async () => {
    await withHistory();
    render(<LibraryImportSettings />);
    const report = await screen.findByTestId("import-report-streaming");
    expect(report).toHaveTextContent("5,210 plays imported");
    expect(report).toHaveTextContent("610 of 1,240 songs are in your library");
    expect(report).not.toHaveTextContent("0 of 0");
    fireEvent.click(within(report).getByRole("button", { name: "Played a lot, not in your library" }));
    expect(route()).toEqual({ name: "import", view: "played" });
    fireEvent.click(within(report).getByRole("button", { name: "See your stats" }));
    expect(route()).toEqual({ name: "stats" });
  });

  it("the import page's report card says the same and counts new plays", async () => {
    await withHistory();
    render(<ImportPage />);
    const card = await screen.findByTestId("report-streaming");
    expect(card).toHaveTextContent("5,210 plays imported");
    expect(card).toHaveTextContent("5,210 new plays added");
    expect(within(card).getByRole("img", { name: "49% found" })).toBeInTheDocument();
  });

  it("says plainly what was left out when a file had nothing usable", async () => {
    await settings();
    vi.spyOn(mock, "importChoose").mockResolvedValue("Downloads/empty_history.json");
    expect(await importFrom("streaming", false)).toBeNull();
    const msg = toasts().map((t) => t.message);
    expect(msg.some((m) => m.includes("no songs Platter could use") && m.includes("plays under 30 seconds"))).toBe(true);
  });
});

describe("Played a lot", () => {
  it("lists songs not in the library, most played first, with plays, time and last played", async () => {
    await withHistory();
    render(<ImportPage view="played" />);
    expect(await screen.findByRole("tab", { name: "Played a lot", selected: true })).toBeInTheDocument();
    await waitFor(() => expect(rows()).toHaveLength(6));
    expect(titles()).toEqual([
      "Paper Boats by Harbor Lights",
      "Neon Driveway by The Long Weekends",
      "Thin Ice Waltz by Cass Morrow",
      "Undertow by Harbor Lights",
      "Glass Orchard by Mira Vale",
      "Field Notes by Osa Lind",
    ]);
    const top = rows()[0];
    expect(top).toHaveTextContent(`48 plays · ${playTime(48 * 214_000)} · last played`);
    expect(top).toHaveTextContent("Low Water");
    expect(within(top).getByRole("button", { name: "Search Bandcamp for Paper Boats" })).toBeInTheDocument();
    expect(screen.getByText(/^6 songs/)).toBeInTheDocument();
  });

  it("shows albums with how much of each is played and already in the library", async () => {
    await withHistory();
    render(<ImportPage view="played" />);
    await waitFor(() => expect(rows().length).toBeGreaterThan(0));
    fireEvent.click(screen.getByRole("radio", { name: "Albums" }));
    await waitFor(() => expect(titles()[0]).toBe("Low Water by Harbor Lights"));
    expect(rows()[0]).toHaveTextContent("65 plays");
    expect(rows()[0]).toHaveTextContent("2 songs played");
    expect(rows()[0]).toHaveTextContent("1 already in your library");
  });

  it("filters by period and fewest plays, kept in the synced settings", async () => {
    await withHistory();
    render(<ImportPage view="played" />);
    await waitFor(() => expect(rows()).toHaveLength(6));
    fireEvent.change(screen.getByRole("combobox", { name: "Period" }), { target: { value: "days90" } });
    await waitFor(() => expect(titles()).toEqual(["Thin Ice Waltz by Cass Morrow", "Field Notes by Osa Lind"]));
    expect(useSettings.getState().settings?.playedPeriod).toBe("days90");
    fireEvent.change(screen.getByRole("combobox", { name: "Fewest plays" }), { target: { value: "1" } });
    await waitFor(() => expect(titles()).toEqual(["Thin Ice Waltz by Cass Morrow", "Field Notes by Osa Lind", "Paper Boats by Harbor Lights", "Undertow by Harbor Lights"]));
    expect((await mock.settingsGet()).playedMinPlays).toBe(1);
    fireEvent.change(screen.getByRole("combobox", { name: "Fewest plays" }), { target: { value: "50" } });
    expect(await screen.findByText("Everything you played 50 times or more in the last 90 days is in your library.")).toBeInTheDocument();
  });

  it("hides a row or a whole artist, and brings them back", async () => {
    await withHistory();
    render(<ImportPage view="played" />);
    await waitFor(() => expect(rows()).toHaveLength(6));
    fireEvent.click(within(rows()[1]).getByRole("button", { name: "Hide Neon Driveway" }));
    await waitFor(() => expect(titles()).not.toContain("Neon Driveway by The Long Weekends"));
    fireEvent.click(within(rows()[0]).getByRole("button", { name: "Hide everything by Harbor Lights" }));
    await waitFor(() => expect(titles()).toEqual(["Thin Ice Waltz by Cass Morrow", "Glass Orchard by Mira Vale", "Field Notes by Osa Lind"]));
    fireEvent.click(screen.getByRole("button", { name: "2 hidden" }));
    const list = screen.getByRole("list", { name: "Hidden from Played a lot" });
    expect(list).toHaveTextContent("Everything by Harbor Lights");
    expect(list).toHaveTextContent("Neon Driveway by The Long Weekends");
    const harbor = within(list).getAllByRole("listitem").find((li) => li.textContent?.includes("Harbor Lights"))!;
    fireEvent.click(within(harbor).getByRole("button", { name: "Show again" }));
    await waitFor(() => expect(titles()).toContain("Paper Boats by Harbor Lights"));
    expect(await mock.importPlayedHidden()).toHaveLength(1);
  });

  it("drops a row when the library gains the song", async () => {
    await withHistory();
    render(<ImportPage view="played" />);
    await waitFor(() => expect(rows()).toHaveLength(6));
    mock.importMock.libraryGained("Osa Lind", "Field Notes");
    act(() => (mock as unknown as { emit: (e: string, p: unknown) => void }).emit("library://changed", null));
    await waitFor(() => expect(rows()).toHaveLength(5));
    expect(titles()).not.toContain("Field Notes by Osa Lind");
  });

  it("points to Library import when no history was imported", async () => {
    await settings();
    render(<ImportPage view="played" />);
    expect(await screen.findByText("No listening history yet")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Import history" }));
    expect(route()).toEqual({ name: "settings" });
  });

  it("a reload that answers late doesn't replace a newer one", async () => {
    await withHistory();
    render(<ImportPage view="played" />);
    await waitFor(() => expect(rows()).toHaveLength(6));
    const real = mock.importPlayed.bind(mock);
    const held: (() => void)[] = [];
    const spy = vi.spyOn(mock, "importPlayed").mockImplementation((args) =>
      real(args).then((p) => new Promise((r) => held.push(() => r(p)))),
    );
    const changed = () => act(() => (mock as unknown as { emit: (e: string, p: unknown) => void }).emit("library://changed", null));
    // First reload: before the library gains a song. Second: after.
    changed();
    await waitFor(() => expect(held).toHaveLength(1));
    mock.importMock.libraryGained("Osa Lind", "Field Notes");
    changed();
    await waitFor(() => expect(held).toHaveLength(2));
    await act(async () => held[1]());
    await waitFor(() => expect(rows()).toHaveLength(5));
    await act(async () => held[0]());
    expect(rows()).toHaveLength(5);
    expect(titles()).not.toContain("Field Notes by Osa Lind");
    spy.mockRestore();
  });

  it("has no tab when the setting is off", async () => {
    await withHistory({ showPlayedALot: false });
    render(<ImportPage view="missing" />);
    expect(await screen.findByRole("heading", { name: "Missing songs" })).toBeInTheDocument();
    expect(screen.queryByRole("tab", { name: "Played a lot" })).toBeNull();
  });
});

describe("requesting from Played a lot", () => {
  it("Get on a song asks for its album, the first one through the typed confirm, then shows its state", async () => {
    await withHistory({ lidarrUrl: "http://192.168.x.x:8686" });
    const user = userEvent.setup();
    render(<ImportPage view="played" />);
    const row = await waitFor(() => rows()[0]);
    await user.click(await within(row).findByRole("button", { name: "Get Paper Boats" }));
    // A few seconds to undo, then the first request ever opens the dialog.
    const dialog = await screen.findByRole("dialog", { name: "Request Low Water" }, { timeout: 8000 });
    expect(await within(dialog).findByText("/downloads/lidarr/Harbor Lights")).toBeInTheDocument();
    // The first request ever needs the album's name typed.
    await user.type(within(dialog).getByRole("textbox"), "Low Water");
    await user.click(within(dialog).getByRole("button", { name: /^Request/ }));
    await waitFor(() => expect(within(rows()[0]).getByText("Looking on your indexers")).toBeInTheDocument());
    expect(mock.lidarr.list()[0].title).toBe("Low Water");
  });

  it("refuses a bootleg-only song without asking Lidarr", async () => {
    await withHistory({ lidarrUrl: "http://192.168.x.x:8686" });
    render(<ImportPage view="played" />);
    await waitFor(() => expect(rows()).toHaveLength(6));
    const glass = rows().find((r) => r.getAttribute("aria-label") === "Glass Orchard by Mira Vale")!;
    fireEvent.click(await within(glass).findByRole("button", { name: "Get Glass Orchard" }));
    await waitFor(() => expect(toasts().some((t) => t.message.includes("bootleg"))).toBe(true), { timeout: 8000 });
    expect(mock.lidarr.list()).toHaveLength(0);
  });

  it("previews each picked album and sends them after one confirm, at most ten", async () => {
    await withHistory({ lidarrUrl: "http://192.168.x.x:8686" });
    // Typed once already, so a batch can go.
    await mock.lidarrRequest({ releaseGroupId: "7a2d0c55-0000-4000-8000-000000000014", title: "Field Notes", artist: "Osa Lind", typed: "Field Notes" });
    const batch = vi.spyOn(mock, "lidarrRequestBatch");
    render(<ImportPage view="played" />);
    await waitFor(() => expect(rows()).toHaveLength(6));
    // Field Notes shows its request already.
    const field = rows().find((r) => r.getAttribute("aria-label") === "Field Notes by Osa Lind")!;
    expect(within(field).getByText("Looking on your indexers")).toBeInTheDocument();
    fireEvent.click(within(rows()[1]).getByRole("checkbox", { name: "Pick Neon Driveway to request" }));
    fireEvent.click(within(rows()[2]).getByRole("checkbox", { name: "Pick Thin Ice Waltz to request" }));
    fireEvent.click(within(rows()[4]).getByRole("checkbox", { name: "Pick Glass Orchard to request" }));
    fireEvent.click(screen.getByRole("button", { name: "Request these 3" }));
    const dialog = await screen.findByRole("dialog", { name: "Request these albums" });
    const list = await within(dialog).findByRole("list", { name: "What will be requested" });
    expect(list).toHaveTextContent("Night Errands");
    expect(list).toHaveTextContent("Into /downloads/lidarr/The Long Weekends");
    expect(list).toHaveTextContent("Winter Rooms");
    expect(list).toHaveTextContent(/Glass Orchard.*Skipped: .*bootleg/);
    expect(batch).not.toHaveBeenCalled();
    fireEvent.click(within(dialog).getByRole("button", { name: "Request 2" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(batch).toHaveBeenCalledTimes(1);
    expect(batch.mock.calls[0][0].map((i) => i.title)).toEqual(["Night Errands", "Winter Rooms"]);
    await waitFor(() => expect(within(rows()[1]).getByText("Looking on your indexers")).toBeInTheDocument());
    expect(toasts().some((t) => t.message === "Requested 2 albums from Lidarr")).toBe(true);
  });

  it("caps a batch at ten in the core too", async () => {
    await withHistory({ lidarrUrl: "http://192.168.x.x:8686" });
    expect(BATCH_MAX).toBe(10);
    const many = Array.from({ length: 11 }, (_, i) => ({ releaseGroupId: `rg-${i}`, title: `A${i}`, artist: "X" }));
    await expect(mock.lidarrRequestBatch(many)).rejects.toMatchObject({ message: "Request up to 10 albums at a time." });
  });
});

describe("Played a lot settings", () => {
  it("live in Library import, on by default, with fewest plays and period", async () => {
    await settings();
    render(<PlayedSettings />);
    const sw = await screen.findByRole("switch", { name: 'Show "Played a lot" on the Missing page' });
    expect(sw).toHaveAttribute("aria-checked", "true");
    expect(screen.getByRole("combobox", { name: "Fewest plays" })).toHaveValue("5");
    expect(screen.getByRole("combobox", { name: "Period" })).toHaveValue("all");
    fireEvent.change(screen.getByRole("combobox", { name: "Period" }), { target: { value: "year" } });
    await waitFor(() => expect(useSettings.getState().settings?.playedPeriod).toBe("year"));
    fireEvent.click(sw);
    await waitFor(() => expect(useSettings.getState().settings?.showPlayedALot).toBe(false));
    expect(screen.queryByRole("combobox", { name: "Period" })).toBeNull();
    expect((await mock.settingsGet()).showPlayedALot).toBe(false);
  });

  it("show up in the Library import section", async () => {
    await settings();
    render(<LibraryImportSettings />);
    const section = await screen.findByTestId("library-import-settings");
    expect(within(section).getByText('Show "Played a lot" on the Missing page')).toBeInTheDocument();
  });
});
