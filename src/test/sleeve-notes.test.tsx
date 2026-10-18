/**
 * Sleeve notes in the UI: the album page's tab (keyboard too), facts with
 * their sources, names opening the credit page, the empty state and how to
 * get more, the Credits dialog from a song's menu, reloading when lookups
 * finish, and the two settings.
 */
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { SongCreditsHost } from "../components/SongCreditsDialog";
import { trackMenuItems } from "../components/TrackRow";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { countryName, factRows, formatSleeveDate, sourcesText } from "../lib/sleeve";
import { DEFAULT_SETTINGS, type Album, type SleeveNotes } from "../lib/types";
import { AlbumPage } from "../pages/AlbumPage";
import { CreditPage } from "../pages/CreditPage";
import { SettingsPage } from "../pages/SettingsPage";
import { useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import { closeSongCredits, openSongCredits } from "../stores/songCredits";
import { ONLINE_EXTRAS } from "./onlineExtras";

const mock = backend as MockBackend;
const route = () => useRouter.getState().entries[useRouter.getState().index].route;

async function setUp(patch: Record<string, unknown> = {}) {
  await mock.settingsSet({ ...DEFAULT_SETTINGS, ...ONLINE_EXTRAS, ...patch });
  useSettings.setState({ settings: null });
  await loadSettings();
}

/** An album whose notes have MusicBrainz facts and credits, and one with nothing known. */
async function albums(): Promise<{ full: Album; notes: SleeveNotes; empty: Album }> {
  let full: Album | null = null;
  let notes: SleeveNotes | null = null;
  let empty: Album | null = null;
  for (const a of mock.fx.albums) {
    const n = await backend.sleeveNotes(a.id);
    if (!full && n.catalogue && n.groups.some((g) => g.role === "producer") && n.tracks.some((t) => t.groups.length)) {
      full = a;
      notes = n;
    }
    if (!empty && n.groups.length === 0 && !n.label && !n.released) empty = a;
    if (full && empty) break;
  }
  return { full: full!, notes: notes!, empty: empty! };
}

async function openSleeveTab(album: Album) {
  const user = userEvent.setup();
  render(<AlbumPage id={album.id} />);
  await screen.findByRole("heading", { level: 1, name: album.title });
  await user.click(screen.getByRole("tab", { name: "Sleeve notes" }));
  return user;
}

beforeEach(async () => {
  useRouter.getState().reset();
  closeSongCredits();
  mock.physical.reset();
  await setUp();
});

describe("words", () => {
  it("writes dates, spans, countries and sources as people do", () => {
    expect(formatSleeveDate("1997", "en-US")).toBe("1997");
    expect(formatSleeveDate("1997-05", "en-US")).toBe("May 1997");
    expect(formatSleeveDate("1997-05-21", "en-US")).toBe("May 21, 1997");
    expect(formatSleeveDate("1996-11/1997-02", "en-US")).toBe("November 1996 to February 1997");
    expect(countryName("GB", "en-US")).toBe("United Kingdom");
    expect(countryName("XW")).toBe("Worldwide");
    expect(sourcesText(["tags", "musicbrainz"])).toBe("your files' tags and MusicBrainz");
    expect(sourcesText(["tags", "plex", "discogs"])).toBe("your files' tags, Plex and Discogs");
  });

  it("lists the facts a sleeve prints, in order, leaving out what's unknown", async () => {
    const { notes } = await albums();
    const rows = factRows(notes);
    expect(rows[0].key).toBe("released");
    expect(rows.map((r) => r.key)).toContain("catalogue");
    expect(rows.every((r) => r.text.length > 0)).toBe(true);
  });
});

describe("the Sleeve notes tab", () => {
  it("sits next to the songs and works from the keyboard", async () => {
    const { full } = await albums();
    const user = userEvent.setup();
    render(<AlbumPage id={full.id} />);
    await screen.findByRole("heading", { level: 1, name: full.title });
    const songs = screen.getByRole("tab", { name: "Songs" });
    expect(songs).toHaveAttribute("aria-selected", "true");
    songs.focus();
    await user.keyboard("{ArrowRight}");
    const sleeve = screen.getByRole("tab", { name: "Sleeve notes" });
    expect(sleeve).toHaveAttribute("aria-selected", "true");
    expect(sleeve).toHaveFocus();
    const panel = await screen.findByRole("tabpanel");
    expect(panel).toHaveAttribute("aria-labelledby", sleeve.id);
    expect(await within(panel).findByRole("heading", { name: "The release" })).toBeInTheDocument();
    expect(within(panel).queryByRole("grid")).toBeNull();
    await user.keyboard("{ArrowLeft}");
    expect(songs).toHaveAttribute("aria-selected", "true");
    await waitFor(() => expect(screen.queryByRole("heading", { name: "The release" })).toBeNull());
  });

  it("shows each fact with where it came from", async () => {
    const { full, notes } = await albums();
    await openSleeveTab(full);
    const release = await screen.findByRole("region", { name: "The release" });
    const label = within(release).getByText("Label").closest(".sleeve-facts__row") as HTMLElement;
    expect(within(label).getByText(notes.label!.value)).toBeInTheDocument();
    expect(within(label).getByText(`From ${sourcesText(notes.label!.sources)}`)).toBeInTheDocument();
    const credits = screen.getByRole("region", { name: "Credits" });
    expect(within(credits).getAllByText("Produced by")[0].tagName).toBe("DT");
    expect(within(credits).getByRole("heading", { name: "Song by song" })).toBeInTheDocument();
    expect(screen.getByText(/Marks show where each fact came from/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Open on MusicBrainz/ })).toBeInTheDocument();
  });

  it("a name opens their credit page, listing what they're credited on by role", async () => {
    const { full, notes } = await albums();
    const user = await openSleeveTab(full);
    const producer = notes.groups.find((g) => g.role === "producer")!.people[0];
    const credits = await screen.findByRole("region", { name: "Credits" });
    await user.click(within(credits).getAllByRole("button", { name: producer.name })[0]);
    expect(route()).toEqual({ name: "credits", person: producer.name });

    render(<CreditPage person={producer.name} />);
    expect(await screen.findByRole("heading", { level: 1, name: producer.name })).toBeInTheDocument();
    const produced = screen.getByRole("region", { name: /^Produced/ });
    await user.click(within(produced).getByRole("button", { name: new RegExp(full.title) }));
    expect(route()).toEqual({ name: "album", id: full.id });
  });

  it("nothing known shows an empty state with how to get more", async () => {
    const { empty } = await albums();
    await openSleeveTab(empty);
    expect(await screen.findByRole("heading", { name: "No sleeve notes yet" })).toBeInTheDocument();
    // Lookups are on and MusicBrainz is on; a Discogs token would add more.
    expect(screen.getByRole("button", { name: "Add a Discogs token" })).toBeInTheDocument();
  });

  it("with lookups off it says how to turn them on", async () => {
    await setUp({ sleeveLookups: false });
    const { empty } = await albums();
    const user = await openSleeveTab(empty);
    await user.click(await screen.findByRole("button", { name: "Turn on online lookups" }));
    expect(route()).toEqual({ name: "settings", row: "sleeveLookups" });
  });

  it("asks again when the core says lookups finished", async () => {
    const { full } = await albums();
    const spy = vi.spyOn(backend, "sleeveNotes");
    await openSleeveTab(full);
    await screen.findByRole("region", { name: "The release" });
    const before = spy.mock.calls.length;
    mock.emit("sleeve://changed", { id: "someone-else" });
    mock.emit("sleeve://changed", { id: full.id });
    await waitFor(() => expect(spy.mock.calls.length).toBe(before + 1));
    spy.mockRestore();
  });

  it("opens on the tab when a link asks for it", async () => {
    const { full } = await albums();
    render(<AlbumPage id={full.id} tab="sleeve" />);
    expect(await screen.findByRole("region", { name: "The release" })).toBeInTheDocument();
  });
});

describe("song credits", () => {
  it("are in a song's menu and open a dialog with that song's credits", async () => {
    const { full, notes } = await albums();
    const song = notes.tracks.find((t) => t.groups.length)!;
    const track = mock.fx.tracks.find((t) => t.id === song.trackId)!;
    const item = trackMenuItems(track).find((i) => i.label === "Credits")!;
    expect(item.when?.() ?? true).toBe(true);
    const user = userEvent.setup();
    render(<SongCreditsHost />);
    item.onSelect!();
    const dialog = await screen.findByRole("dialog", { name: `Credits for ${track.title}` });
    const person = song.groups[0].people[0];
    expect(await within(dialog).findByRole("button", { name: person.name })).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Album sleeve notes" }));
    expect(route()).toEqual({ name: "album", id: full.id, tab: "sleeve" });
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("a name in the dialog closes it and opens the credit page", async () => {
    const { notes } = await albums();
    const song = notes.tracks.find((t) => t.groups.length)!;
    const track = mock.fx.tracks.find((t) => t.id === song.trackId)!;
    const user = userEvent.setup();
    render(<SongCreditsHost />);
    openSongCredits(track);
    const dialog = await screen.findByRole("dialog");
    const person = song.groups[0].people[0];
    await user.click(await within(dialog).findByRole("button", { name: person.name }));
    expect(route()).toEqual({ name: "credits", person: person.name });
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});

describe("settings", () => {
  it("both are on, in Library, and the lookups row follows Show sleeve notes", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    const library = await screen.findByRole("region", { name: "Library" });
    const show = within(library).getByRole("switch", { name: "Show sleeve notes" });
    const lookups = within(library).getByRole("switch", { name: "Look up credits online" });
    expect(show).toBeChecked();
    expect(lookups).toBeChecked();
    await user.click(lookups);
    await waitFor(async () => expect((await backend.settingsGet()).sleeveLookups).toBe(false));
    await user.click(show);
    await waitFor(async () => expect((await backend.settingsGet()).showSleeveNotes).toBe(false));
    await waitFor(() => expect(within(library).queryByRole("switch", { name: "Look up credits online" })).toBeNull());
  });

  it("Show sleeve notes off hides the tab and the Credits item", async () => {
    const { full } = await albums();
    await setUp({ showSleeveNotes: false });
    render(<AlbumPage id={full.id} />);
    await screen.findByRole("heading", { level: 1, name: full.title });
    expect(screen.queryByRole("tab", { name: "Sleeve notes" })).toBeNull();
    const track = mock.fx.tracks.find((t) => t.albumId === full.id)!;
    expect(trackMenuItems(track).find((i) => i.label === "Credits")!.when!()).toBe(false);
  });
});
