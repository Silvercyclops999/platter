/**
 * The listening diary in the UI: month sections and the year view, the
 * entry editor (rating, note, delete), filters and search, export, Log this
 * album and First heard on album pages, the off state and the settings.
 */
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ContextMenuHost } from "../components/ContextMenu";
import { backend } from "../lib/backend";
import { byMonth, longDate, monthKey, monthLabel, playedLine, whereLine } from "../lib/diary";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS, type Album, type DiaryItem } from "../lib/types";
import { AlbumPage } from "../pages/AlbumPage";
import { DiaryPage } from "../pages/DiaryPage";
import { FirstHeard } from "../pages/FirstHeard";
import { StarInput } from "../pages/StarRating";
import { SettingsPage } from "../pages/SettingsPage";
import { nav, useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";
import { ONLINE_EXTRAS } from "./onlineExtras";

const mock = backend as MockBackend;

async function setUp(patch: Record<string, unknown> = {}) {
  await mock.settingsSet({ ...DEFAULT_SETTINGS, ...ONLINE_EXTRAS, ...patch });
  useSettings.setState({ settings: null });
  await loadSettings();
}

function page() {
  const user = userEvent.setup();
  render(
    <>
      <DiaryPage />
      <ContextMenuHost />
    </>,
  );
  return user;
}

async function all(): Promise<DiaryItem[]> {
  return (await backend.diaryEntries()).entries;
}

beforeEach(async () => {
  useRouter.getState().reset();
  useToasts.setState({ toasts: [] });
  mock.diary.reset();
  mock.lastDiaryExport = null;
  await setUp();
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("words and dates", () => {
  it("groups entries by local month, newest first", async () => {
    const entries = await all();
    const months = byMonth(entries);
    expect(months[0].key).toBe(monthKey(entries[0].at));
    expect(months.flatMap((m) => m.entries)).toEqual(entries);
    expect(monthLabel("2021-03")).toBe("March 2021");
    expect(longDate(new Date(2021, 2, 3, 20).getTime())).toBe("March 3, 2021");
  });

  it("says how much was played and where", () => {
    const e = { auto: true, songsPlayed: 9, songsTotal: 10, minutes: 41, listenedOn: "headphones", output: "HD 600", device: "Den PC" } as DiaryItem;
    expect(playedLine(e)).toBe("Played 9 of 10 songs, 41 min");
    expect(playedLine({ ...e, songsPlayed: 10 })).toBe("Played all 10 songs, 41 min");
    expect(playedLine({ ...e, auto: false })).toBe("Logged by hand");
    expect(whereLine(e)).toBe("On headphones, Den PC");
    expect(whereLine({ ...e, listenedOn: null })).toBe("On HD 600, Den PC");
    expect(whereLine({ ...e, listenedOn: null, output: null, device: null })).toBe("");
  });
});

describe("the diary page", () => {
  it("shows month sections with dated entries: sleeve, title, artist, rating and note", async () => {
    const entries = await all();
    page();
    expect(await screen.findByRole("heading", { level: 1, name: "Listening diary" })).toBeInTheDocument();
    const first = byMonth(entries)[0];
    const section = await screen.findByRole("region", { name: monthLabel(first.key) });
    const rows = within(section).getAllByRole("listitem");
    expect(rows).toHaveLength(first.entries.length);
    const e = first.entries[0];
    expect(within(rows[0]).getByRole("heading", { level: 3, name: e.title })).toBeInTheDocument();
    expect(within(rows[0]).getByText(e.artist, { exact: false })).toBeInTheDocument();
    if (e.rating != null) expect(within(rows[0]).getByRole("img", { name: `Rated ${e.rating} out of 5` })).toBeInTheDocument();
    expect(screen.getByText(`${entries.length} entries since`, { exact: false })).toHaveTextContent("Only on this computer.");
  });

  it("the year view lays out a year's covers by month with counts", async () => {
    const entries = await all();
    const user = page();
    await screen.findByRole("heading", { level: 1, name: "Listening diary" });
    await user.click(screen.getByRole("tab", { name: "Year" }));
    const year = new Date(entries[0].at).getFullYear();
    const grid = await screen.findByRole("list", { name: `${year} by month` });
    const inYear = entries.filter((e) => new Date(e.at).getFullYear() === year);
    const month = new Date(entries[0].at).getMonth();
    const count = inYear.filter((e) => new Date(e.at).getMonth() === month).length;
    const name = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"][month];
    expect(within(grid).getByRole("button", { name: `${name}: ${count === 1 ? "1 album" : `${count} albums`}` })).toBeEnabled();
    expect(within(grid).getAllByRole("listitem").filter((li) => li.classList.contains("diary-cell"))).toHaveLength(12);
    expect(screen.getByText(`${inYear.length === 1 ? "1 album" : `${inYear.length} albums`} in ${year}.`)).toBeInTheDocument();
    // The year before has the older entries.
    await user.click(screen.getByRole("button", { name: "Previous year" }));
    expect(await screen.findByRole("list", { name: `${year - 1} by month` })).toBeInTheDocument();
    // A month opens the entries at that month.
    await user.click(screen.getByRole("button", { name: "Next year" }));
    await user.click(await screen.findByRole("button", { name: new RegExp(`^${name}:`) }));
    expect(await screen.findByRole("region", { name: `${name} ${year}` })).toBeInTheDocument();
    expect(screen.getByRole("combobox", { name: "Year" })).toHaveValue(String(year));
  });

  it("adds a rating and a note to an entry", async () => {
    const entries = await all();
    const target = entries.find((e) => e.rating == null && !e.note)!;
    const user = page();
    // Edit buttons follow the entries' order (an album can have several).
    const buttons = await screen.findAllByRole("button", { name: /^Edit the entry for/ });
    await user.click(buttons[entries.indexOf(target)]);
    const dialog = await screen.findByRole("dialog", { name: `Diary entry for ${target.title}` });
    const stars = within(dialog).getByRole("slider", { name: "Rating" });
    expect(stars).toHaveAttribute("aria-valuetext", "No rating");
    stars.focus();
    await user.keyboard("4{ArrowRight}");
    expect(stars).toHaveAttribute("aria-valuetext", "4.5 stars");
    await user.type(within(dialog).getByRole("textbox", { name: "Note" }), "The quiet one in the middle.");
    expect(within(dialog).getByText("28 / 500")).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    await waitFor(async () => {
      const saved = (await all()).find((e) => e.id === target.id)!;
      expect(saved.rating).toBe(4.5);
      expect(saved.note).toBe("The quiet one in the middle.");
    });
    expect(await screen.findByText("The quiet one in the middle.")).toBeInTheDocument();
  });

  it("deletes an entry after asking once more", async () => {
    const [first] = await all();
    const user = page();
    const buttons = await screen.findAllByRole("button", { name: `Edit the entry for ${first.title}` });
    await user.click(buttons[0]);
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Delete entry" }));
    await user.click(within(dialog).getByRole("button", { name: "Delete for good" }));
    await waitFor(async () => expect((await all()).some((e) => e.id === first.id)).toBe(false));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("filters to rated entries, entries with notes and a year, and searches notes", async () => {
    const entries = await all();
    const user = page();
    await screen.findByRole("heading", { level: 1, name: "Listening diary" });
    const count = () => document.querySelectorAll(".diary-entry").length;
    await waitFor(() => expect(count()).toBe(entries.length));
    await user.click(screen.getByRole("button", { name: "With notes" }));
    await waitFor(() => expect(count()).toBe(entries.filter((e) => e.note).length));
    await user.click(screen.getByRole("button", { name: "Rated" }));
    await waitFor(() => expect(count()).toBe(entries.filter((e) => e.rating != null).length));
    await user.click(screen.getByRole("button", { name: "All" }));
    const year = new Date(entries[entries.length - 1].at).getFullYear();
    await user.selectOptions(screen.getByRole("combobox", { name: "Year" }), String(year));
    await waitFor(() => expect(count()).toBe(entries.filter((e) => new Date(e.at).getFullYear() === year).length));
    await user.selectOptions(screen.getByRole("combobox", { name: "Year" }), "");
    await user.type(screen.getByRole("searchbox", { name: "Search your notes" }), "rainy");
    await waitFor(() => expect(count()).toBe(entries.filter((e) => e.note.toLowerCase().includes("rainy")).length));
    await user.type(screen.getByRole("searchbox", { name: "Search your notes" }), " nothingmatches");
    expect(await screen.findByRole("heading", { name: "No entries match" })).toBeInTheDocument();
  });

  it("exports what's shown as text or CSV through the core", async () => {
    const spy = vi.spyOn(backend, "diaryExport");
    const user = page();
    await screen.findAllByRole("button", { name: /^Edit the entry for/ });
    await user.click(screen.getByRole("button", { name: "Export" }));
    await user.click(await screen.findByRole("menuitem", { name: "Export as CSV" }));
    await waitFor(() => expect(spy).toHaveBeenCalledWith("csv", expect.objectContaining({})));
    await waitFor(() => expect(useToasts.getState().toasts.some((t) => t.message === "Saved Listening diary.csv")).toBe(true));
    expect(mock.lastDiaryExport?.format).toBe("csv");
  });

  it("an empty diary says how entries arrive", async () => {
    mock.diary.clear();
    page();
    expect(await screen.findByRole("heading", { name: "No entries yet" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Export" })).toBeDisabled();
  });

  it("opened from Log this album's toast, it opens that entry's editor", async () => {
    const album = mock.fx.albums.find((a) => a.section !== "demos")!;
    const e = await backend.diaryLogAlbum(album.id);
    render(<DiaryPage edit={e.id} />);
    expect(await screen.findByRole("dialog", { name: `Diary entry for ${album.title}` })).toBeInTheDocument();
  });

  it("the year view starts on the newest year of the whole diary, whatever the filter", async () => {
    mock.diary.clear();
    const [a, b] = mock.fx.albums.filter((x) => x.section !== "demos");
    const now = new Date();
    const y = now.getFullYear();
    const rated = await backend.diaryLogAlbum(a.id);
    await backend.diaryEntryEdit(rated.id, { rating: 4, note: "", at: new Date(y - 3, 5, 10, 20).getTime() });
    const unrated = await backend.diaryLogAlbum(b.id);
    await backend.diaryEntryEdit(unrated.id, { rating: null, note: "", at: new Date(y - 2, 2, 4, 20).getTime() });
    const user = page();
    await screen.findByRole("heading", { level: 1, name: "Listening diary" });
    await user.click(screen.getByRole("button", { name: "Rated" }));
    await user.click(screen.getByRole("tab", { name: "Year" }));
    expect(await screen.findByRole("list", { name: `${y - 2} by month` })).toBeInTheDocument();
    expect(screen.getByText(`Nothing logged in ${y - 2}.`)).toBeInTheDocument();
  });

  it("an entry's sleeve and title open the album, with one stop for the keyboard", async () => {
    const [first] = await all();
    const user = page();
    const row = (await screen.findAllByRole("listitem")).find((li) => li.classList.contains("diary-entry"))!;
    const title = within(row).getByRole("button", { name: first.title });
    const sleeve = row.querySelector<HTMLButtonElement>(".diary-entry__sleeve")!;
    expect(sleeve).toHaveAttribute("tabindex", "-1");
    expect(sleeve).toHaveAttribute("aria-hidden", "true");
    await user.click(sleeve);
    let r = useRouter.getState();
    expect(r.entries[r.index].route).toEqual({ name: "album", id: first.albumId });
    await user.click(title);
    r = useRouter.getState();
    expect(r.entries[r.index].route).toEqual({ name: "album", id: first.albumId });
  });

  it("the editor won't save an empty or future date, and says why", async () => {
    const [first] = await all();
    const user = page();
    await user.click((await screen.findAllByRole("button", { name: `Edit the entry for ${first.title}` }))[0]);
    const dialog = await screen.findByRole("dialog");
    const date = dialog.querySelector<HTMLInputElement>('input[type="date"]')!;
    const save = within(dialog).getByRole("button", { name: "Save" });
    expect(save).toBeEnabled();
    fireEvent.change(date, { target: { value: "" } });
    expect(save).toBeDisabled();
    expect(within(dialog).getByText("Enter a date to save.")).toBeInTheDocument();
    expect(date).toHaveAccessibleDescription("Enter a date to save.");
    const next = new Date();
    next.setDate(next.getDate() + 2);
    const ahead = `${next.getFullYear()}-${String(next.getMonth() + 1).padStart(2, "0")}-${String(next.getDate()).padStart(2, "0")}`;
    fireEvent.change(date, { target: { value: ahead } });
    expect(save).toBeDisabled();
    expect(within(dialog).getByText("That date is in the future.")).toBeInTheDocument();
    fireEvent.change(date, { target: { value: "2020-05-01" } });
    expect(save).toBeEnabled();
    expect(within(dialog).queryByText("That date is in the future.")).toBeNull();
  });

  it("the editor names its rating and note count for screen readers", async () => {
    const [first] = await all();
    const user = page();
    await user.click((await screen.findAllByRole("button", { name: `Edit the entry for ${first.title}` }))[0]);
    const dialog = await screen.findByRole("dialog");
    const stars = within(dialog).getByRole("slider", { name: "Rating" });
    const label = within(dialog).getByText("Rating", { selector: "span" });
    expect(stars).toHaveAttribute("aria-labelledby", label.id);
    expect(stars).not.toHaveAttribute("aria-label");
    const note = within(dialog).getByRole("textbox", { name: "Note" });
    expect(note).toHaveAccessibleDescription(`${[...first.note].length} / 500`);
  });

  it("Delete asks once more on the same button, says so, and takes it back on leaving", async () => {
    const [first] = await all();
    const user = page();
    await user.click((await screen.findAllByRole("button", { name: `Edit the entry for ${first.title}` }))[0]);
    const dialog = await screen.findByRole("dialog");
    const del = within(dialog).getByRole("button", { name: "Delete entry" });
    await user.click(del);
    expect(del).toHaveFocus();
    expect(del).toHaveTextContent("Delete for good");
    const live = dialog.querySelector('[aria-live="assertive"]')!;
    expect(live).toHaveTextContent(`Press again to delete the entry for ${first.title} for good.`);
    // Moving away takes the question back.
    await user.tab();
    expect(del).toHaveTextContent("Delete entry");
    expect(live).toHaveTextContent("");
    expect((await all()).some((e) => e.id === first.id)).toBe(true);
  });

  it("off, it offers to turn the diary on", async () => {
    await setUp({ keepDiary: false });
    const user = page();
    expect(await screen.findByRole("heading", { name: "The listening diary is off" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Keep a listening diary" }));
    await waitFor(async () => expect((await backend.settingsGet()).keepDiary).toBe(true));
  });
});

/** An album the mock says was first heard long ago, and one never played. */
function albums(): { heard: Album; never: Album } {
  const list = mock.fx.albums;
  const logged = new Set(mock.diary.list(null).entries.map((e) => e.albumId));
  const heard = list.find((_, i) => i % 2 === 0)!;
  const never = list.find((a, i) => i % 2 === 1 && !logged.has(a.id))!;
  return { heard, never };
}

async function openAlbum(album: Album) {
  const user = userEvent.setup();
  render(
    <>
      <AlbumPage id={album.id} />
      <ContextMenuHost />
    </>,
  );
  await screen.findByRole("heading", { level: 1, name: album.title });
  return user;
}

describe("album pages", () => {
  it("show when the album was first heard", async () => {
    const { heard } = albums();
    const at = await backend.albumFirstHeard(heard.id);
    await openAlbum(heard);
    expect(await screen.findByText(longDate(at!))).toHaveTextContent(`First heard: ${longDate(at!)}`);
  });

  it("say nothing for an album never played, or with the diary off", async () => {
    const { never, heard } = albums();
    await openAlbum(never);
    await screen.findAllByRole("row");
    expect(screen.queryByText("First heard:")).toBeNull();
    await setUp({ keepDiary: false });
    await openAlbum(heard);
    await screen.findAllByRole("row");
    expect(screen.queryByText("First heard:")).toBeNull();
  });

  it("Log this album in the menu adds an entry, and the toast opens it to add a note", async () => {
    const { never } = albums();
    const user = await openAlbum(never);
    await user.click(screen.getByRole("button", { name: `More options for ${never.title}` }));
    await user.click(within(await screen.findByRole("menu")).getByRole("menuitem", { name: "Log this album" }));
    await waitFor(() => expect(useToasts.getState().toasts.some((t) => t.message === `Logged ${never.title} in your diary`)).toBe(true));
    const entry = (await all()).find((e) => e.albumId === never.id)!;
    expect(entry.auto).toBe(false);
    useToasts.getState().toasts.find((t) => t.action?.label === "Add a note")!.action!.run();
    const r = useRouter.getState();
    expect(r.entries[r.index].route).toEqual({ name: "diary", edit: entry.id });
    // First heard now shows the logged date.
    expect(await screen.findByText("First heard:", { exact: false })).toBeInTheDocument();
  });

  it("the menu has no Log this album with the diary off", async () => {
    await setUp({ keepDiary: false });
    const { never } = albums();
    const user = await openAlbum(never);
    await user.click(screen.getByRole("button", { name: `More options for ${never.title}` }));
    const menu = await screen.findByRole("menu");
    expect(within(menu).queryByRole("menuitem", { name: "Log this album" })).toBeNull();
  });
});

describe("first heard and stars", () => {
  it("First heard drops the last album's date as soon as the album changes", async () => {
    const { heard, never } = albums();
    const at = await backend.albumFirstHeard(heard.id);
    const { rerender } = render(<FirstHeard albumId={heard.id} />);
    expect(await screen.findByText(longDate(at!))).toBeInTheDocument();
    // The next album's answer is slow to come.
    vi.spyOn(backend, "albumFirstHeard").mockReturnValue(new Promise(() => {}));
    rerender(<FirstHeard albumId={never.id} />);
    expect(screen.queryByText("First heard:")).toBeNull();
  });

  it("stars pick on a click, not a press, and step a whole star with Page Up and Page Down", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    const { rerender } = render(<StarInput value={2} onChange={onChange} label="Rating" />);
    const stars = screen.getByRole("slider", { name: "Rating" });
    expect(stars).toHaveAttribute("aria-orientation", "horizontal");
    vi.spyOn(stars, "getBoundingClientRect").mockReturnValue({ left: 0, width: 100, top: 0, height: 20, right: 100, bottom: 20, x: 0, y: 0, toJSON: () => ({}) });
    fireEvent.pointerDown(stars, { clientX: 75 });
    expect(onChange).not.toHaveBeenCalled();
    fireEvent.click(stars, { clientX: 75 });
    expect(onChange).toHaveBeenLastCalledWith(4);
    stars.focus();
    await user.keyboard("{PageUp}");
    expect(onChange).toHaveBeenLastCalledWith(3);
    rerender(<StarInput value={0.5} onChange={onChange} label="Rating" />);
    await user.keyboard("{PageDown}");
    expect(onChange).toHaveBeenLastCalledWith(null);
  });
});

describe("settings", () => {
  it("Keep a listening diary is on in Library, adds albums on its own, and isn't synced", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    const library = await screen.findByRole("region", { name: "Library" });
    const adv = within(library).queryByRole("button", { name: /^Advanced/ });
    const keep = within(library).queryByRole("switch", { name: "Keep a listening diary" });
    if (!keep && adv) await user.click(adv);
    const toggle = within(library).getByRole("switch", { name: "Keep a listening diary" });
    expect(toggle).toBeChecked();
    const auto = within(library).getByRole("checkbox", { name: "Add albums played start to finish automatically" });
    const sync = within(library).getByRole("checkbox", { name: "Sync the diary to my devices" });
    expect(auto).toBeChecked();
    expect(sync).not.toBeChecked();
    await user.click(sync);
    await waitFor(async () => expect((await backend.settingsGet()).syncDiary).toBe(true));
    await user.click(auto);
    await waitFor(async () => expect((await backend.settingsGet()).diaryAuto).toBe(false));
    await user.click(toggle);
    await waitFor(async () => expect((await backend.settingsGet()).keepDiary).toBe(false));
    await waitFor(() => expect(within(library).queryByRole("checkbox", { name: "Sync the diary to my devices" })).toBeNull());
  });

  it("the app menu and the route lead to the diary", () => {
    nav.go({ name: "diary" });
    const r = useRouter.getState();
    expect(r.entries[r.index].route).toEqual({ name: "diary" });
  });
});
