/**
 * Library doctor in the UI: the page lists findings of every kind with a
 * count, filters by kind, shows "Why?" with each song's tags, ignores and
 * brings back findings, opens the app's own fixes (Change cover, Fix match,
 * merge with a confirm, the better copy, a Lidarr request when ready), and
 * follows the settings (off, each check, the small cover size). The app
 * menu shows the count.
 */
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import type { DoctorFinding } from "../lib/types";
import { DEFAULT_SETTINGS } from "../lib/types";
import { DoctorPage, countLine, fixLabel } from "../pages/DoctorPage";
import { SettingsPage } from "../pages/SettingsPage";
import { TopBar } from "../shell/TopBar";
import { useApp } from "../stores/app";
import { badgeText } from "../stores/doctor";
import { reloadEditAccess, usePlexEdit } from "../stores/plexEdit";
import { useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";

const mock = backend as MockBackend;

async function settings(patch: Record<string, unknown> = {}) {
  await mock.settingsSet({ ...DEFAULT_SETTINGS, ...patch });
  useSettings.setState({ settings: null });
  await loadSettings();
}

beforeEach(async () => {
  mock.doctor.reset();
  mock.plexEdit.shared = false;
  usePlexEdit.setState({ dialog: null });
  await reloadEditAccess();
  useToasts.setState({ toasts: [] });
  useRouter.getState().reset();
  useApp.setState({ auth: { state: "signedIn", username: "listener", needsServerPick: false } });
  await settings();
});

const lastRoute = () => {
  const e = useRouter.getState().entries;
  return e[e.length - 1].route;
};
const section = (name: RegExp | string) => screen.getByRole("region", { name });
const findingTitled = (text: string | RegExp) => screen.getByRole("heading", { level: 3, name: text }).closest("li")!;

describe("helpers", () => {
  it("counts and labels", () => {
    expect(countLine(0)).toBe("Nothing to fix");
    expect(countLine(1)).toBe("1 thing to fix");
    expect(countLine(15)).toBe("15 things to fix");
    expect(badgeText(7)).toBe("7");
    expect(badgeText(240)).toBe("99+");
    const f = { fix: "openAlbum", kind: "lowQuality", others: [] } as unknown as DoctorFinding;
    expect(fixLabel(f)).toEqual({ label: "Open the better copy", edit: false });
    expect(fixLabel({ ...f, kind: "duplicate" })).toEqual({ label: "Open album", edit: false });
    expect(fixLabel({ ...f, fix: "cover" }).edit).toBe(true);
    expect(fixLabel({ ...f, fix: "mergeAlbums", others: [{ id: "1", title: "x", thumb: null }] }).label).toBe("Merge into one");
  });
});

describe("the library doctor page", () => {
  it("lists findings of every kind, grouped, with the count and when it last checked", async () => {
    render(<DoctorPage />);
    expect(await screen.findByText("15 things to fix")).toBeInTheDocument();
    expect(screen.getByText(/Checked 5 minutes ago/)).toBeInTheDocument();
    const headings = screen.getAllByRole("heading", { level: 2 }).map((h) => h.textContent?.replace(/\d+$/, "").trim());
    expect(headings).toEqual([
      "Duplicate files",
      "Covers",
      "Album tags that differ",
      "Albums split in two",
      "Missing album artist",
      "Low-quality copies",
      "Albums that weren't matched",
    ]);
    expect(within(section(/^Covers/)).getAllByRole("listitem")).toHaveLength(3);
    expect(within(section(/^Covers/)).getByText("No cover")).toBeInTheDocument();
    // Some checks wait for file tags still being read.
    expect(screen.getByText(/still being read/)).toBeInTheDocument();
  });

  it("filters by kind with chips", async () => {
    const user = userEvent.setup();
    render(<DoctorPage />);
    await screen.findByText("15 things to fix");
    const chips = screen.getByRole("group", { name: "Show findings" });
    await user.click(within(chips).getByRole("button", { name: /Albums split in two/ }));
    expect(screen.getAllByRole("heading", { level: 2 })).toHaveLength(1);
    expect(within(chips).getByRole("button", { name: /Albums split in two/ })).toHaveAttribute("aria-pressed", "true");
    await user.click(within(chips).getByRole("button", { name: "All" }));
    expect(screen.getAllByRole("heading", { level: 2 })).toHaveLength(7);
  });

  it("Why? shows the evidence, and See the tags shows each song's tags with the odd one out marked", async () => {
    const user = userEvent.setup();
    render(<DoctorPage />);
    await screen.findByText("15 things to fix");
    const dupe = within(section(/^Duplicate files/)).getAllByRole("listitem")[0];
    const why = within(dupe).getByRole("button", { name: "Why?" });
    expect(why).toHaveAttribute("aria-expanded", "false");
    await user.click(why);
    expect(why).toHaveAttribute("aria-expanded", "true");
    expect(within(dupe).getByText(/twice with the same ISRC/)).toBeInTheDocument();

    const tags = findingTitled("The songs don't agree on the album name and year");
    await user.click(within(tags).getByRole("button", { name: "See the tags" }));
    const table = within(tags).getByRole("table");
    expect(within(table).getAllByRole("row").length).toBeGreaterThan(2);
    const odd = table.querySelectorAll(".doctor-tags__odd");
    expect(odd.length).toBe(2);
    expect(odd[0].textContent).toMatch(/\(Deluxe\)$/);
  });

  it("Ignore takes a finding out of the count, and Show again brings it back", async () => {
    const user = userEvent.setup();
    render(<DoctorPage />);
    await screen.findByText("15 things to fix");
    const first = within(section(/^Covers/)).getAllByRole("listitem")[0];
    const title = within(first).getByRole("heading", { level: 3 }).textContent!;
    await user.click(within(first).getByRole("button", { name: "Ignore" }));
    expect(await screen.findByText("14 things to fix")).toBeInTheDocument();
    expect([...mock.doctor.ignored.values()].map((i) => i.title)).toEqual([title]);
    await user.click(screen.getByRole("button", { name: "Show ignored (1)" }));
    const ignored = screen.getByRole("region", { name: "Ignored" });
    expect(within(ignored).getByText(title)).toBeInTheDocument();
    await user.click(within(ignored).getByRole("button", { name: "Show again" }));
    expect(await screen.findByText("15 things to fix")).toBeInTheDocument();
    expect(mock.doctor.ignored.size).toBe(0);
  });

  it("Change cover and Fix match open the app's own edit dialogs for the album", async () => {
    const user = userEvent.setup();
    render(<DoctorPage />);
    await screen.findByText("15 things to fix");
    const cover = within(section(/^Covers/)).getAllByRole("listitem")[0];
    await user.click(within(cover).getByRole("button", { name: "Change cover" }));
    const d = usePlexEdit.getState().dialog!;
    expect(d.name).toBe("cover");
    expect(d.target.kind).toBe("album");
    const unmatched = within(section(/^Albums that weren't matched/)).getAllByRole("listitem")[0];
    await user.click(within(unmatched).getByRole("button", { name: "Fix match" }));
    expect(usePlexEdit.getState().dialog!.name).toBe("match");
    // Nothing was written by opening them.
    expect(mock.doctor.fixed.size).toBe(0);
  });

  it("a fix that runs takes its finding away", async () => {
    render(<DoctorPage />);
    await screen.findByText("15 things to fix");
    const f = mock.doctor.report().findings.find((x) => x.kind === "unmatched")!;
    await backend.plexEditMatch(f.albumId, "album", { guid: "plex://album/1", name: f.album, artist: f.artist, year: 2016, score: 100 });
    expect(await screen.findByText("14 things to fix")).toBeInTheDocument();
  });

  it("without editing rights the fix opens the album and Why? says why", async () => {
    const user = userEvent.setup();
    mock.plexEdit.shared = true;
    await reloadEditAccess();
    render(<DoctorPage />);
    await screen.findByText("15 things to fix");
    const cover = within(section(/^Covers/)).getAllByRole("listitem")[0];
    await waitFor(() => expect(within(cover).queryByRole("button", { name: "Change cover" })).not.toBeInTheDocument());
    await user.click(within(cover).getByRole("button", { name: "Why?" }));
    expect(within(cover).getByText(/can't edit this server/)).toBeInTheDocument();
    await user.click(within(cover).getByRole("button", { name: "Open album" }));
    expect(lastRoute().name).toBe("album");
  });

  it("low-quality copies open the better copy, and look for one through Lidarr only when it's ready", async () => {
    const user = userEvent.setup();
    render(<DoctorPage />);
    await screen.findByText("15 things to fix");
    const low = within(section(/^Low-quality copies/)).getAllByRole("listitem")[0];
    expect(within(low).queryByRole("button", { name: "Find a better copy" })).not.toBeInTheDocument();
    const f = mock.doctor.report().findings.find((x) => x.kind === "lowQuality")!;
    await user.click(within(low).getByRole("button", { name: "Open the better copy" }));
    expect(lastRoute()).toEqual({ name: "album", id: f.others[0].id });
  });

  it("with Lidarr ready, Find a better copy goes through the request picker", async () => {
    const user = userEvent.setup();
    mock.lidarr.reset("ready");
    await settings({ lidarrUrl: "http://lidarr.test:8686" });
    const pick = vi.spyOn(backend, "lidarrPick").mockResolvedValue({ group: null, artist: "x", refusal: "titleClash", reason: "You have an album by that name already." } as never);
    render(<DoctorPage />);
    await screen.findByText("15 things to fix");
    const low = within(section(/^Low-quality copies/)).getAllByRole("listitem")[0];
    await user.click(await within(low).findByRole("button", { name: "Find a better copy" }));
    const f = mock.doctor.report().findings.find((x) => x.kind === "lowQuality")!;
    expect(pick).toHaveBeenCalledWith({ artist: f.artist, artistId: f.artistId, title: f.album, song: false });
    await waitFor(() => expect(useToasts.getState().toasts.some((t) => t.message === "You have an album by that name already.")).toBe(true));
    pick.mockRestore();
  });

  it("follows the settings: a check off, the cover size, and the doctor off", async () => {
    const user = userEvent.setup();
    await settings({ doctorChecks: { ...DEFAULT_SETTINGS.doctorChecks, duplicates: false }, doctorCoverPx: 1000 });
    const { unmount } = render(<DoctorPage />);
    // Two duplicates gone, one more small cover (800 px) counted.
    expect(await screen.findByText("14 things to fix")).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: /^Duplicate files/ })).not.toBeInTheDocument();
    expect(within(section(/^Covers/)).getAllByRole("listitem")).toHaveLength(4);
    unmount();
    await settings({ libraryDoctor: false });
    render(<DoctorPage />);
    expect(await screen.findByText(/The library doctor is off/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Check now" })).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Turn on" }));
    await waitFor(() => expect(useSettings.getState().settings?.libraryDoctor).toBe(true));
  });

  it("Check now starts a check and the line says so", async () => {
    const user = userEvent.setup();
    const run = vi.spyOn(backend, "doctorRun");
    render(<DoctorPage />);
    await screen.findByText("15 things to fix");
    await user.click(screen.getByRole("button", { name: "Check now" }));
    expect(run).toHaveBeenCalled();
    expect(await screen.findByText("Checking your library…")).toBeInTheDocument();
    run.mockRestore();
  });

  it("merging a split album asks first, then merges and the finding goes", async () => {
    const user = userEvent.setup();
    const merge = vi.spyOn(backend, "plexEditMergeAlbums");
    render(<DoctorPage />);
    await screen.findByText("15 things to fix");
    const split = within(section(/^Albums split in two/)).getAllByRole("listitem")[0];
    const f = mock.doctor.report().findings.find((x) => x.kind === "splitAlbum")!;
    await user.click(within(split).getByRole("button", { name: "Merge into one" }));
    const dialog = screen.getByRole("dialog");
    expect(within(dialog).getByRole("heading", { name: `Merge the two parts of ${f.album}?` })).toBeInTheDocument();
    expect(merge).not.toHaveBeenCalled();
    await user.click(within(dialog).getByRole("button", { name: "Merge" }));
    expect(merge).toHaveBeenCalledWith(f.albumId, [f.others[0].id]);
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(await screen.findByText("14 things to fix")).toBeInTheDocument();
    await waitFor(() => expect(useToasts.getState().toasts.some((t) => t.message.startsWith("Merged the two parts of") && t.action?.label === "Undo")).toBe(true));
    merge.mockRestore();
  });
});

describe("where it opens from", () => {
  it("the app menu shows the count and opens the page", async () => {
    const user = userEvent.setup();
    render(<TopBar />);
    await user.click(screen.getByRole("button", { name: /^App menu/ }));
    const item = await screen.findByRole("menuitem", { name: "Library doctor, 15 to fix" });
    expect(within(item).getByText("15")).toBeInTheDocument();
    await user.click(item);
    expect(lastRoute()).toEqual({ name: "doctor" });
  });

  it("the app menu leaves it out while the doctor is off", async () => {
    const user = userEvent.setup();
    await settings({ libraryDoctor: false });
    render(<TopBar />);
    await user.click(screen.getByRole("button", { name: /^App menu/ }));
    expect(screen.queryByRole("menuitem", { name: /Library doctor/ })).not.toBeInTheDocument();
  });

  it("Settings > Library doctor has its switch, each check and the small cover size", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    await user.type(screen.getByRole("searchbox", { name: "Search settings" }), "library doctor");
    const run = screen.getByRole("switch", { name: "Check the library in the background" });
    expect(run).toBeChecked();
    const checks = screen.getByRole("list", { name: "Library doctor checks" });
    const switches = within(checks).getAllByRole("switch");
    expect(switches).toHaveLength(8);
    for (const s of switches) expect(s).toBeChecked();
    await user.click(within(checks).getByRole("switch", { name: "Missing or small covers" }));
    await waitFor(() => expect(useSettings.getState().settings?.doctorChecks.covers).toBe(false));
    expect(useSettings.getState().settings?.doctorChecks.duplicates).toBe(true);
    await user.click(await screen.findByRole("button", { name: /Open library doctor/ }));
    expect(lastRoute()).toEqual({ name: "doctor" });
  });

  it("Settings: the small cover size is one of the offered sizes", async () => {
    await settings({ doctorCoverPx: 800 });
    render(<SettingsPage />);
    const row = document.querySelector('[data-setting="doctorCoverPx"]')!;
    expect(row).not.toBeNull();
    expect(within(row as HTMLElement).getByText("800 pixels")).toBeInTheDocument();
    await expect(mock.settingsSet({ doctorCoverPx: 601 })).rejects.toBeTruthy();
  });
});
