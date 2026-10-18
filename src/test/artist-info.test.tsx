import { ONLINE_EXTRAS } from "./onlineExtras";
import { act, fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS, type ArtistInfo } from "../lib/types";
import { ArtistPage } from "../pages/ArtistPage";
import { SettingsPage } from "../pages/SettingsPage";
import { keyNote } from "../pages/ArtistInfoSettings";
import { aboutFacts, aboutText, artistSections, bannerPath, bioParagraphs, bioSegments, galleryPaths, headerVariant, listenersLine, NO_BIO, photoPath, relatedGroups } from "../pages/artist";
import { useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";

// These render the whole Settings page; a loaded CI machine needs more than 15 s.
vi.setConfig({ testTimeout: 30_000 });

const mock = backend as MockBackend;
const artistAt = (i: number) => mock.fx.artists[i];

beforeEach(async () => {
  await backend.settingsSet({ ...DEFAULT_SETTINGS, ...ONLINE_EXTRAS });
  for (const s of ["lastfm", "fanart", "audiodb"] as const) await backend.artistInfoKeySet(s, null);
  await loadSettings();
  useToasts.setState({ toasts: [] });
});

async function turnOn(source: "lastfm" | "fanart", key = "good-key") {
  await backend.artistInfoKeySet(source, key);
  await backend.settingsSet(source === "lastfm" ? { infoLastfm: true } : { infoFanart: true });
  await loadSettings();
}

describe("artist info decisions", () => {
  const info = (p: Partial<ArtistInfo>): ArtistInfo => ({
    id: "1",
    bio: null,
    bioSource: null,
    origin: null,
    yearsActive: null,
    members: [],
    tags: [],
    listeners: null,
    scrobbles: null,
    listenersFrom: null,
    related: [],
    photo: null,
    banner: null,
    logo: null,
    credits: [],
    gallery: [],
    links: [],
    bioLinks: [],
    ...p,
  });

  it("uses an outside banner only when Plex has no background art", () => {
    expect(bannerPath({ art: "/plex/art" }, info({ banner: "/ext/b" }))).toBe("/plex/art");
    expect(bannerPath({ art: null }, info({ banner: "/ext/b" }))).toBe("/ext/b");
    expect(headerVariant({ art: null }, "/ext/b")).toBe("banner");
    expect(headerVariant({ art: null }, null)).toBe("photo");
    expect(photoPath({ thumb: "/plex/t" }, info({ photo: "/ext/p" }))).toBe("/plex/t");
    expect(photoPath({ thumb: null }, info({ photo: "/ext/p" }))).toBe("/ext/p");
  });

  it("shows the outside sources' bio pick, else Plex's", () => {
    expect(aboutText("Plex", info({ bio: "Wiki" }))).toBe("Wiki");
    expect(aboutText("Plex", info({}))).toBe("Plex");
    expect(aboutText(null, null)).toBe(null);
  });

  it("formats the listener line and splits paragraphs", () => {
    expect(listenersLine(48210)).toBe("48,210 listeners on Last.fm");
    expect(listenersLine(1)).toBe("1 listener on Last.fm");
    expect(listenersLine(812, "listenbrainz")).toBe("812 listeners on ListenBrainz");
    expect(relatedGroups([
      { name: "A", artistId: null, relation: "Subgroup" },
      { name: "B", artistId: "2", relation: "Member of" },
      { name: "C", artistId: null, relation: "Subgroup" },
    ]).map(([rel, people]) => [rel, people.map((p) => p.name)])).toEqual([["Subgroup", ["A", "C"]], ["Member of", ["B"]]]);
    expect(bioParagraphs("One.\n\nTwo.\nThree.")).toEqual(["One.", "Two.", "Three."]);
    expect(aboutFacts(info({ origin: "Town", yearsActive: "1990–present" }))).toEqual([
      { label: "From", value: "Town" },
      { label: "Years active", value: "1990–present" },
    ]);
    expect(aboutFacts(null)).toEqual([]);
  });

  it("shows About for listeners alone", () => {
    const base = { popular: [], sections: [], about: null, similar: [] };
    expect(artistSections(base)).toEqual([]);
    expect(artistSections({ ...base, listeners: 10 })).toEqual(["about"]);
    // Without a bio, a photo or plays still keep the card (it says there's no bio).
    expect(artistSections({ ...base, photo: "/p" })).toEqual(["about"]);
    expect(artistSections({ ...base, plays: 3 })).toEqual(["about"]);
  });

  it("links each library artist the bio names once, whole words only", () => {
    const links = [
      { text: "Mara Quill", artistId: "5" },
      { text: "Owls", artistId: "6" },
    ];
    const used = new Set<string>();
    expect(bioSegments("With Mara Quill and the Owlsome Owls.", links, used)).toEqual([
      { text: "With " },
      { text: "Mara Quill", artistId: "5" },
      { text: " and the Owlsome " },
      { text: "Owls", artistId: "6" },
      { text: "." },
    ]);
    // Later paragraphs don't link them again.
    expect(bioSegments("Mara Quill again.", links, used)).toEqual([{ text: "Mara Quill again." }]);
    expect(bioSegments("Plain.", [], new Set())).toEqual([{ text: "Plain." }]);
  });

  it("gallery: the core's list, else the one photo there is", () => {
    expect(galleryPaths({ thumb: "/t" }, info({ gallery: ["/t", "/ext/a"] }))).toEqual(["/t", "/ext/a"]);
    expect(galleryPaths({ thumb: "/t" }, null)).toEqual(["/t"]);
    expect(galleryPaths({ thumb: null }, info({ photo: "/ext/p" }))).toEqual(["/ext/p"]);
    expect(galleryPaths({ thumb: null }, null)).toEqual([]);
  });

  it("names the service that refused a key", () => {
    const keys = { lastfm: { saved: true, rejected: true }, fanart: { saved: false, rejected: false }, audiodb: { saved: false, rejected: false }, listenbrainz: { saved: true, rejected: true } };
    expect(keyNote("lastfm", keys)).toBe("Last.fm rejected the key.");
    expect(keyNote("fanart", keys)).toBeUndefined();
    expect(keyNote("listenbrainz", keys)).toBe("ListenBrainz rejected the token.");
  });
});

describe("artist page with outside info", () => {
  it("shows the Wikipedia bio on the card and everything in the About dialog", async () => {
    const user = userEvent.setup();
    const open = vi.spyOn(backend, "openInfoLink");
    const a = artistAt(2);
    render(<ArtistPage id={a.id} />);
    const card = await screen.findByRole("button", { name: `About ${a.title}` });
    expect(within(card).getByText(new RegExp(`^${a.title} is a music act formed in`))).toHaveClass("is-clamped");
    // Without Last.fm, the stats line is ListenBrainz's listeners.
    expect(within(card).getByText(/listeners on ListenBrainz$/)).toHaveClass("about-card__stats");

    await user.click(card);
    const dialog = screen.getByRole("dialog", { name: `About ${a.title}` });
    expect(within(dialog).getAllByText(/./, { selector: ".about-dialog__bio p" }).length).toBe(3);
    expect(within(dialog).getByText("From")).toBeInTheDocument();
    expect(within(dialog).getByText("Years active")).toBeInTheDocument();
    // Credits: Wikipedia text with its licence, Wikidata facts.
    const sources = within(dialog).getByRole("list", { name: "Sources" });
    expect(within(sources).getByRole("button", { name: "From Wikipedia" })).toBeInTheDocument();
    expect(within(sources).getByRole("button", { name: "CC BY-SA 4.0" })).toBeInTheDocument();
    expect(within(sources).getByRole("button", { name: "Facts from Wikidata" })).toBeInTheDocument();
    expect(within(sources).queryByText("Data from Last.fm")).toBeNull();
    await user.click(within(sources).getByRole("button", { name: "From Wikipedia" }));
    expect(open).toHaveBeenCalledWith(expect.stringMatching(/^https:\/\/en\.wikipedia\.org\/wiki\//));

    // A member who's in the library links to their page.
    const member = artistAt(5);
    await user.click(within(dialog).getByRole("button", { name: member.title }));
    expect(screen.queryByRole("dialog")).toBeNull();
    const r = useRouter.getState();
    expect(r.entries[r.index].route).toEqual({ name: "artist", id: member.id });
    open.mockRestore();
  });

  it("the dialog: gallery with arrows, stats and links beside the bio, names linked, closes outside", async () => {
    const user = userEvent.setup();
    await turnOn("lastfm");
    await turnOn("fanart");
    const open = vi.spyOn(backend, "openInfoLink");
    const a = artistAt(2);
    render(<ArtistPage id={a.id} />);
    await vi.waitFor(async () => {
      const i = await backend.artistInfo(a.id);
      expect(i.gallery.length).toBeGreaterThan(2);
    });
    await user.click(await screen.findByRole("button", { name: `About ${a.title}` }));
    const dialog = screen.getByRole("dialog", { name: `About ${a.title}` });

    // Gallery: one photo at a time, arrows and arrow keys step through.
    const gallery = await within(dialog).findByRole("group", { name: /^Photos of / });
    expect(gallery).toHaveAccessibleName(expect.stringMatching(/, 1 of \d+$/));
    await user.click(within(dialog).getByRole("button", { name: "Next photo" }));
    expect(gallery).toHaveAccessibleName(expect.stringMatching(/, 2 of \d+$/));
    fireEvent.keyDown(gallery, { key: "ArrowLeft" });
    expect(gallery).toHaveAccessibleName(expect.stringMatching(/, 1 of \d+$/));
    fireEvent.keyDown(gallery, { key: "ArrowLeft" });
    // Back past the first: the last photo.
    expect(gallery.getAttribute("aria-label")).toMatch(/, (\d+) of \1$/);

    // Side column: Last.fm numbers and your plays, facts, links.
    const side = within(dialog).getByRole("complementary", { name: "Stats and links" });
    expect(within(side).getByText("listeners on Last.fm")).toBeInTheDocument();
    expect(within(side).getByText(/plays? in your library/)).toBeInTheDocument();
    expect(within(side).getByText("Years active")).toBeInTheDocument();
    const links = within(side).getByRole("list", { name: "Links" });
    await user.click(within(links).getByRole("button", { name: "Instagram" }));
    expect(open).toHaveBeenCalledWith(expect.stringMatching(/^https:\/\/www\.instagram\.com\//), a.id);
    expect(within(links).getByRole("button", { name: "Wikipedia" })).toBeInTheDocument();

    // A library artist the bio names links to their page.
    const info = await backend.artistInfo(a.id);
    expect(info.bioLinks.length).toBe(1);
    const named = within(dialog).getByRole("button", { name: info.bioLinks[0].text });
    expect(named).toHaveClass("about-bio-link");
    await user.click(named);
    expect(screen.queryByRole("dialog")).toBeNull();
    const r = useRouter.getState();
    expect(r.entries[r.index].route).toEqual({ name: "artist", id: info.bioLinks[0].artistId });
    open.mockRestore();
  });

  it("closes with a click outside, Esc or the close button", async () => {
    const user = userEvent.setup();
    const a = artistAt(2);
    render(<ArtistPage id={a.id} />);
    const card = await screen.findByRole("button", { name: `About ${a.title}` });
    await user.click(card);
    fireEvent.mouseDown(document.querySelector(".dialog-backdrop")!);
    expect(screen.queryByRole("dialog")).toBeNull();
    await user.click(card);
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).toBeNull();
    await user.click(card);
    await user.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Close" }));
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("keeps a long Plex bio and credits no outside text", async () => {
    const user = userEvent.setup();
    const a = artistAt(0); // long Plex bio
    render(<ArtistPage id={a.id} />);
    await user.click(await screen.findByRole("button", { name: `About ${a.title}` }));
    const dialog = screen.getByRole("dialog");
    expect(within(dialog).queryByRole("button", { name: "From Wikipedia" })).toBeNull();
    expect(within(dialog).getByRole("button", { name: "Facts from Wikidata" })).toBeInTheDocument();
  });

  it("adds the Last.fm listener line when Last.fm is on with a key", async () => {
    await turnOn("lastfm");
    const a = artistAt(2);
    render(<ArtistPage id={a.id} />);
    const lines = await screen.findAllByText(/listeners on Last\.fm$/);
    expect(lines.length).toBe(2); // header and About card
    // On the card the listener line takes the plays line's place, in bold over the photo.
    const card = screen.getByRole("button", { name: `About ${a.title}` });
    expect(within(card).getByText(/listeners on Last\.fm$/)).toHaveClass("about-card__stats");
    expect(within(card).queryByText(/plays in your library$/)).toBeNull();
    expect(card.querySelector(".about-card__bg")).not.toBeNull();
    await userEvent.setup().click(screen.getByRole("button", { name: `About ${a.title}` }));
    const dialog = screen.getByRole("dialog");
    expect(within(dialog).getByRole("button", { name: "Data from Last.fm" })).toBeInTheDocument();
    expect(within(dialog).getByRole("list", { name: "Tags" })).toBeInTheDocument();
  });

  it("each source off hides its data and credits", async () => {
    await turnOn("lastfm");
    await backend.settingsSet({ infoWikipedia: false, infoLastfm: false, infoMusicbrainz: false, infoListenbrainzStats: false });
    await loadSettings();
    const a = artistAt(2);
    render(<ArtistPage id={a.id} />);
    await screen.findByRole("heading", { level: 1, name: a.title });
    // No Plex bio, no outside sources: the card stays, saying so.
    const card = await screen.findByRole("button", { name: `About ${a.title}` });
    expect(within(card).getByText(NO_BIO)).toBeInTheDocument();
    expect(screen.queryByText(/listeners on/)).toBeNull();
    // Nothing on: the stats line is the plays in the library.
    expect(within(card).getByText(/plays? in your library$/)).toHaveClass("about-card__stats");
  });

  it("MusicBrainz members carry their years, related acts link, ListenBrainz fills the listener line", async () => {
    const user = userEvent.setup();
    const a = artistAt(2);
    render(<ArtistPage id={a.id} />);
    expect(await screen.findAllByText(/listeners on ListenBrainz$/)).toHaveLength(2); // header and About card
    await user.click(screen.getByRole("button", { name: `About ${a.title}` }));
    const dialog = screen.getByRole("dialog", { name: `About ${a.title}` });
    const side = within(dialog).getByRole("complementary", { name: "Stats and links" });
    expect(within(side).getByText("listeners on ListenBrainz")).toBeInTheDocument();
    expect(side.querySelectorAll(".about-members__years").length).toBeGreaterThan(0);
    expect(within(side).getByText("Subgroup")).toBeInTheDocument();
    const sources = within(dialog).getByRole("list", { name: "Sources" });
    expect(within(sources).getByRole("button", { name: "Members and relationships from MusicBrainz" })).toBeInTheDocument();
    expect(within(sources).getByRole("button", { name: "Listener numbers from ListenBrainz" })).toBeInTheDocument();
    // A related act in the library links to its page.
    const info = await backend.artistInfo(a.id);
    const kin = info.related.find((r) => r.artistId)!;
    await user.click(within(side).getByRole("button", { name: kin.name }));
    const r = useRouter.getState();
    expect(r.entries[r.index].route).toEqual({ name: "artist", id: kin.artistId });
  });

  it("the MusicBrainz and ListenBrainz stats switches are on by default and synced", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    const sources = row("artistInfoSources");
    const mb = await within(sources).findByRole("checkbox", { name: "MusicBrainz (release dates, credits and members)" });
    const lb = within(sources).getByRole("checkbox", { name: "ListenBrainz (listener numbers)" });
    expect(mb).toBeChecked();
    expect(lb).toBeChecked();
    await user.click(mb);
    await user.click(lb);
    expect(useSettings.getState().settings?.infoMusicbrainz).toBe(false);
    expect(useSettings.getState().settings?.infoListenbrainzStats).toBe(false);
    expect((await backend.albumCredits(mock.fx.albums[0].id)).state).toBe("off");
  });

  it("uses the Fanart.tv banner only when Plex has no background art", async () => {
    await turnOn("fanart");
    const noArt = artistAt(1);
    const { container, unmount } = render(<ArtistPage id={noArt.id} />);
    await screen.findByRole("heading", { level: 1, name: noArt.title });
    await vi.waitFor(() => expect(container.querySelector("header[data-variant]")).toHaveAttribute("data-variant", "banner"));
    unmount();
    // A key that's turned off: back to the round photo.
    await backend.settingsSet({ infoFanart: false });
    await loadSettings();
    const r = render(<ArtistPage id={noArt.id} />);
    await screen.findByRole("heading", { level: 1, name: noArt.title });
    expect(r.container.querySelector("header[data-variant]")).toHaveAttribute("data-variant", "photo");
  });

  it("reloads when the core says more info arrived", async () => {
    const a = artistAt(2);
    render(<ArtistPage id={a.id} />);
    await screen.findByRole("button", { name: `About ${a.title}` });
    const spy = vi.spyOn(mock, "artistInfo");
    act(() => mock.emit("artistinfo://changed", { id: artistAt(4).id }));
    expect(spy).not.toHaveBeenCalled();
    act(() => mock.emit("artistinfo://changed", { id: a.id }));
    expect(spy).toHaveBeenCalledWith(a.id);
    spy.mockRestore();
  });
});

describe("Artist info settings", () => {
  it("has the rows on the Artist info sources page with Wikipedia on and the keyed sources waiting for keys", async () => {
    render(<SettingsPage />);
    const group = await screen.findByRole("region", { name: "Integrations" });
    expect(within(group).getAllByRole("heading", { level: 3, name: "Artist info sources" })[0]).toBeInTheDocument();
    expect(within(group).getByRole("checkbox", { name: "Wikipedia and Wikidata (bios and facts)" })).toBeChecked();
    expect(await within(group).findByLabelText("Last.fm API key")).toBeInTheDocument();
    expect(within(group).getByLabelText("Fanart.tv API key")).toBeInTheDocument();
    expect(within(group).getByLabelText("TheAudioDB API key")).toBeInTheDocument();
  });

  it("saving a good key turns the source on; removing it turns it off", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    await user.type(await screen.findByLabelText("Last.fm API key"), "my-key");
    await user.click(within(row("infoLastfm")).getByRole("button", { name: "Save key" }));
    const sw = await within(row("infoLastfm")).findByRole("switch", { name: "Last.fm" });
    expect(sw).toBeChecked();
    expect(useSettings.getState().settings?.infoLastfm).toBe(true);
    // The key is never shown again.
    expect(screen.queryByDisplayValue("my-key")).toBeNull();
    await user.click(within(row("infoLastfm")).getByRole("button", { name: "Remove key" }));
    expect(await screen.findByLabelText("Last.fm API key")).toBeInTheDocument();
    expect(useSettings.getState().settings?.infoLastfm).toBe(false);
  });

  it("a refused key shows an error on its row and stays off", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    await user.type(await screen.findByLabelText("Fanart.tv API key"), "bad-key");
    await user.click(within(row("infoFanart")).getByRole("button", { name: "Save key" }));
    expect(await within(row("infoFanart")).findByRole("alert")).toHaveTextContent("Fanart.tv rejected the key.");
    expect(within(row("infoFanart")).getByRole("switch", { name: "Fanart.tv" })).not.toBeChecked();
    expect(useSettings.getState().settings?.infoFanart).toBe(false);
  });

  it("ListenBrainz is on without a token; a token saves without touching the switch", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    const sw = await within(row("artistInfoSources")).findByRole("checkbox", { name: "ListenBrainz (popular songs)" });
    expect(sw).toBeChecked();
    await user.type(await screen.findByLabelText("ListenBrainz user token"), "tok");
    await user.click(within(row("infoListenbrainzToken")).getByRole("button", { name: "Save token" }));
    expect(await within(row("infoListenbrainzToken")).findByRole("button", { name: "Remove token" })).toBeInTheDocument();
    expect(useSettings.getState().settings?.infoListenbrainz).toBe(true);
    await user.click(within(row("infoListenbrainzToken")).getByRole("button", { name: "Remove token" }));
    expect(await screen.findByLabelText("ListenBrainz user token")).toBeInTheDocument();
    expect(useSettings.getState().settings?.infoListenbrainz).toBe(true);
    await user.click(sw);
    expect(useSettings.getState().settings?.infoListenbrainz).toBe(false);
  });

  it("a refused token shows on its row", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    await user.type(await screen.findByLabelText("ListenBrainz user token"), "bad-token");
    await user.click(within(row("infoListenbrainzToken")).getByRole("button", { name: "Save token" }));
    expect(await within(row("infoListenbrainzToken")).findByRole("alert")).toHaveTextContent("ListenBrainz rejected the token.");
  });

  it("clears the artist info cache", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    await user.click(await screen.findByRole("button", { name: "Clear artist info" }));
    await vi.waitFor(() => expect(useToasts.getState().toasts.map((t) => t.message)).toContain("Artist info cleared"));
  });
});

describe("app states", () => {
  it("info sources down: the page keeps Plex data and shows no error", async () => {
    const spy = vi.spyOn(mock, "artistInfo").mockRejectedValue({ kind: "network", message: "offline" });
    const a = artistAt(0);
    render(<ArtistPage id={a.id} />);
    const card = await screen.findByRole("button", { name: `About ${a.title}` });
    // Plex's own bio, nothing else.
    expect(within(card).getByText(new RegExp(`^${a.title} `))).toBeInTheDocument();
    expect(screen.queryByText(/couldn't/i)).toBeNull();
    expect(useToasts.getState().toasts).toEqual([]);
    spy.mockRestore();
  });
});

function row(key: string): HTMLElement {
  return document.querySelector(`[data-setting="${key}"]`) as HTMLElement;
}
