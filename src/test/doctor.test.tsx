/**
 * Library doctor in the UI: the page lists findings of every kind with a
 * count, filters by kind, shows "Why?" with each song's tags, ignores and
 * brings back findings, opens the app's own fixes (Change cover, Fix match,
 * merge with a confirm, the better copy, a Lidarr request when ready), and
 * follows the settings (off, each check, the small cover size). A quiet
 * line under Your Library shows the count, and Settings > Library opens it.
 */
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import type { DoctorFinding } from "../lib/types";
import { DEFAULT_SETTINGS } from "../lib/types";
import { DoctorPage, countLine, fixLabel, withGone } from "../pages/DoctorPage";
import { doctorChanges, doctorValueProblem } from "../lib/tagEdit";
import type { TagEditFile } from "../lib/types";
import { SettingsPage } from "../pages/SettingsPage";
import { DoctorNote } from "../shell/Sidebar";
import { useApp } from "../stores/app";
import { badgeText } from "../stores/doctor";
import { reloadEditAccess, usePlexEdit } from "../stores/plexEdit";
import { useRouter } from "../stores/router";
import { useTagEdit } from "../stores/tagEdit";
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
  mock.tagEdit.resetDisk();
  mock.tagEdit.failNext = null;
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

describe("tag fix helpers", () => {
  const file = (trackId: string, values: Record<string, string>, stamp: string | null = "s"): TagEditFile =>
    ({ trackId, albumId: "a", serverPath: null, localPath: null, fileName: `${trackId}.flac`, format: "flac", id3Version: null, values, keys: [], stamp, error: stamp ? null : "Gone", durationMs: 1 }) as TagEditFile;

  it("changes only files and fields that differ; a year matches a date in it; unreadable files are kept", () => {
    const files = [file("1", { album: "Night Drive", year: "2016-05-31" }), file("2", { album: "Night Drive (Deluxe)", year: "2017" }), file("3", {}, null)];
    const { changes, kept } = doctorChanges(files, { album: "Night Drive", year: "2016" });
    expect(changes.map((c) => `${c.trackId}:${c.field}:${c.before}>${c.after}`)).toEqual(["2:album:Night Drive (Deluxe)>Night Drive", "2:year:2017>2016"]);
    expect(kept).toEqual([{ trackId: "3", fileName: "3.flac", reason: "Gone" }]);
    // A full date is compared whole.
    expect(doctorChanges([file("1", { year: "2016-05-31" })], { year: "2016-06-01" }).changes).toHaveLength(1);
    // An empty value changes nothing.
    expect(doctorChanges(files, { albumArtist: "  " }).changes).toEqual([]);
  });

  it("says what's wrong with a value", () => {
    expect(doctorValueProblem("albumArtist", " ")).toMatch(/Enter a value/);
    expect(doctorValueProblem("year", "20l6")).toMatch(/2019/);
    expect(doctorValueProblem("year", "2019-05-31")).toBeNull();
    expect(doctorValueProblem("album", "Night Drive")).toBeNull();
  });

  it("puts ignored lines back where they were, in order", () => {
    const f = (id: string) => ({ id }) as DoctorFinding;
    const ids = (rows: ReturnType<typeof withGone>) => rows.map((r) => `${r.f.id}${r.gone ? "*" : ""}`);
    // X, A, B: A ignored after X, then B after A.
    expect(ids(withGone([f("X")], [{ f: f("A"), after: "X" }, { f: f("B"), after: "A" }]))).toEqual(["X", "A*", "B*"]);
    // The first two, one after the other.
    expect(ids(withGone([f("C")], [{ f: f("A"), after: null }, { f: f("B"), after: "A" }]))).toEqual(["A*", "B*", "C"]);
    // Back in the list: no line.
    expect(ids(withGone([f("A"), f("C")], [{ f: f("A"), after: null }]))).toEqual(["A", "C"]);
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

  it("Why? shows the evidence, and each song's tags with the odd song first and marked", async () => {
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
    await user.click(within(tags).getByRole("button", { name: "Why?" }));
    const table = within(tags).getByRole("table");
    const rows = within(table).getAllByRole("row");
    expect(rows.length).toBeGreaterThan(2);
    const odd = table.querySelectorAll(".doctor-tags__odd");
    expect(odd.length).toBe(2);
    expect(odd[0].textContent).toMatch(/\(Deluxe\) \(differs\)$/);
    // The odd song comes first, under the header, and is marked.
    expect(rows[1]).toHaveClass("doctor-tags__row--odd");
    expect(rows[1].querySelector(".doctor-tags__odd")).not.toBeNull();
    expect(rows[2]).not.toHaveClass("doctor-tags__row--odd");
  });

  it("Fix the tags shows each file's change, saves only what's ticked, and the finding goes", async () => {
    await settings({ musicFolder: "/Volumes/Music", musicFolderServer: "/data/music" });
    const user = userEvent.setup();
    const save = vi.spyOn(backend, "tagEditSave");
    render(<DoctorPage />);
    await screen.findByText("15 things to fix");
    const f = mock.doctor.report().findings.find((x) => x.title === "The songs don't agree on the album name and year")!;
    await user.click(within(findingTitled(f.title)).getByRole("button", { name: "Fix the tags" }));
    const dialog = await screen.findByRole("dialog", { name: "Make the tags match" });
    expect(await within(dialog).findByLabelText("Album")).toHaveValue(f.album);
    expect(within(dialog).getByLabelText("Year")).toHaveValue(String(f.set!.year));
    expect(within(dialog).queryByLabelText("Album artist")).not.toBeInTheDocument();
    // Only the odd song changes: its album name and its year.
    const changes = await within(dialog).findAllByRole("checkbox");
    expect(changes).toHaveLength(2);
    expect(within(dialog).getByText(`${f.album} (Deluxe)`)).toBeInTheDocument();
    expect(save).not.toHaveBeenCalled();
    // Untick the year: only the album name is written.
    await user.click(within(dialog).getByRole("checkbox", { name: /Change Year/ }));
    await user.click(within(dialog).getByRole("button", { name: "Change 1 file" }));
    await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
    const req = save.mock.calls[0][0];
    expect(req.label).toMatch(/^Library doctor: /);
    expect(req.files).toHaveLength(1);
    expect(req.files[0].edits).toEqual({ album: f.album });
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    await waitFor(() => expect(useToasts.getState().toasts.some((t) => t.message === "Saved 1 file" && t.action?.label === "Undo")).toBe(true));
    // The year still differs, so the finding stays; fixing that too clears it.
    expect(mock.doctor.report().findings.some((x) => x.id === f.id)).toBe(true);
    await user.click(within(findingTitled(f.title)).getByRole("button", { name: "Fix the tags" }));
    const again = await screen.findByRole("dialog", { name: "Make the tags match" });
    expect(await within(again).findAllByRole("checkbox")).toHaveLength(1);
    await user.click(within(again).getByRole("button", { name: "Change 1 file" }));
    expect(await screen.findByText("14 things to fix")).toBeInTheDocument();
    save.mockRestore();
  });

  it("a missing album artist is filled in on every song without one, with the value the user picks", async () => {
    await settings({ musicFolder: "/Volumes/Music", musicFolderServer: "/data/music" });
    const user = userEvent.setup();
    const save = vi.spyOn(backend, "tagEditSave");
    render(<DoctorPage />);
    await screen.findByText("15 things to fix");
    const f = mock.doctor.report().findings.find((x) => x.kind === "albumArtist")!;
    await user.click(within(findingTitled(f.title)).getByRole("button", { name: "Fix the tags" }));
    const dialog = await screen.findByRole("dialog", { name: "Set the album artist" });
    const input = await within(dialog).findByLabelText("Album artist");
    expect(input).toHaveValue(f.artist);
    const n = f.table.filter((r) => !r.albumArtist).length;
    expect(await within(dialog).findAllByRole("checkbox")).toHaveLength(n);
    await user.clear(input);
    // Nothing to write: it says so and won't save.
    expect(within(dialog).getByRole("alert")).toHaveTextContent("Album artist: Enter a value first.");
    expect(within(dialog).getByRole("button", { name: /^Change/ })).toBeDisabled();
    await user.type(input, "Various Artists");
    // Now every song changes, the one that had an album artist too.
    expect(within(dialog).getAllByRole("checkbox")).toHaveLength(f.table.length);
    await user.click(within(dialog).getByRole("button", { name: `Change ${f.table.length} files` }));
    await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
    expect(save.mock.calls[0][0].files.every((x) => x.edits.albumArtist === "Various Artists")).toBe(true);
    save.mockRestore();
  });

  it("without a music folder, Fix the tags says so and goes to set it up", async () => {
    const user = userEvent.setup();
    render(<DoctorPage />);
    await screen.findByText("15 things to fix");
    const f = mock.doctor.report().findings.find((x) => x.kind === "albumArtist")!;
    await user.click(within(findingTitled(f.title)).getByRole("button", { name: "Fix the tags" }));
    const dialog = await screen.findByRole("dialog", { name: "Set the album artist" });
    expect(await within(dialog).findByRole("status")).toHaveTextContent(/music folder/i);
    expect(within(dialog).queryAllByRole("checkbox")).toHaveLength(0);
    await user.click(within(dialog).getByRole("button", { name: "Set up music folder" }));
    expect(lastRoute()).toEqual({ name: "settings", page: "tags" });
  });

  it("Edit all tags opens the full tag editor on the album, at the field", async () => {
    await settings({ musicFolder: "/Volumes/Music", musicFolderServer: "/data/music" });
    const user = userEvent.setup();
    render(<DoctorPage />);
    await screen.findByText("15 things to fix");
    const f = mock.doctor.report().findings.find((x) => x.kind === "albumArtist")!;
    await user.click(within(findingTitled(f.title)).getByRole("button", { name: "Fix the tags" }));
    const dialog = await screen.findByRole("dialog", { name: "Set the album artist" });
    await user.click(within(dialog).getByRole("button", { name: "Edit all tags" }));
    expect(useTagEdit.getState().editor).toEqual({ albumId: f.albumId, trackIds: [], focus: "albumArtist" });
    useTagEdit.setState({ editor: null });
  });

  it("a save the file refuses keeps the dialog open with why", async () => {
    await settings({ musicFolder: "/Volumes/Music", musicFolderServer: "/data/music" });
    const user = userEvent.setup();
    render(<DoctorPage />);
    await screen.findByText("15 things to fix");
    const f = mock.doctor.report().findings.find((x) => x.kind === "albumArtist")!;
    await user.click(within(findingTitled(f.title)).getByRole("button", { name: "Fix the tags" }));
    const dialog = await screen.findByRole("dialog", { name: "Set the album artist" });
    await within(dialog).findAllByRole("checkbox");
    mock.tagEdit.failNext = "locked";
    await user.click(within(dialog).getByRole("button", { name: /^Change \d+ files?$/ }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(/open in another program/);
    expect(mock.doctor.report().findings.some((x) => x.id === f.id)).toBe(true);
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
    // A slim line stays where it was, with Undo focused, and no toast covers the next finding.
    const line = within(section(/^Covers/)).getAllByRole("listitem")[0];
    expect(line).toHaveTextContent(`Ignored “${title}” on every device.`);
    expect(within(line).getByRole("button", { name: "Undo" })).toHaveFocus();
    expect(useToasts.getState().toasts).toHaveLength(0);
    // Ignore the next one too: its line goes after the first.
    const next = within(section(/^Covers/)).getAllByRole("listitem")[1];
    const nextTitle = within(next).getByRole("heading", { level: 3 }).textContent!;
    await user.click(within(next).getByRole("button", { name: "Ignore" }));
    expect(await screen.findByText("13 things to fix")).toBeInTheDocument();
    const lines = within(section(/^Covers/)).getAllByRole("listitem");
    expect(lines[0]).toHaveTextContent(title);
    expect(lines[1]).toHaveTextContent(`Ignored “${nextTitle}”`);
    // Undo brings that one back in its place.
    await user.click(within(lines[1]).getByRole("button", { name: "Undo" }));
    expect(await screen.findByText("14 things to fix")).toBeInTheDocument();
    expect(within(section(/^Covers/)).getAllByRole("heading", { level: 3 })[0]).toHaveTextContent(nextTitle);
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
    // Each part says how many songs it has and where it starts.
    expect(within(dialog).getByText(f.albumNote!)).toBeInTheDocument();
    expect(within(dialog).getByText(f.others[0].note!)).toBeInTheDocument();
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
  it("the line under Your Library shows the count and opens the page", async () => {
    const user = userEvent.setup();
    render(<DoctorNote />);
    const item = await screen.findByRole("button", { name: "Library doctor, 15 to fix" });
    expect(within(item).getByText("15")).toBeInTheDocument();
    await user.click(item);
    expect(lastRoute()).toEqual({ name: "doctor" });
  });

  it("the line stays away while the doctor is off", async () => {
    await settings({ libraryDoctor: false });
    render(<DoctorNote />);
    await new Promise((r) => setTimeout(r, 50));
    expect(screen.queryByRole("button", { name: /Library doctor/ })).not.toBeInTheDocument();
  });

  it("Settings > Library has a row that opens it", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    await user.click(await screen.findByRole("button", { name: "Open Library doctor" }));
    expect(lastRoute()).toEqual({ name: "doctor" });
  });

  it("Settings > Library doctor has its switch, each check and the small cover size", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    await user.type(screen.getByRole("searchbox", { name: "Search settings" }), "library doctor");
    const run = screen.getByRole("switch", { name: "Check the library in the background" });
    expect(run).toBeChecked();
    const checks = screen.getByRole("list", { name: "Library doctor checks" });
    const switches = within(checks).getAllByRole("switch");
    expect(switches).toHaveLength(9);
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
