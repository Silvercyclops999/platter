import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS, type ArtistInfo } from "../lib/types";
import { ArtistPage } from "../pages/ArtistPage";
import { SettingsPage } from "../pages/SettingsPage";
import { keyNote } from "../pages/ArtistInfoSettings";
import { aboutFacts, aboutText, artistSections, bannerPath, bioParagraphs, headerVariant, listenersLine, photoPath } from "../pages/artist";
import { useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";

const mock = backend as MockBackend;
const artistAt = (i: number) => mock.fx.artists[i];

beforeEach(async () => {
  await backend.settingsSet({ ...DEFAULT_SETTINGS });
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
    photo: null,
    banner: null,
    logo: null,
    credits: [],
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
    // Without Last.fm, the stats line is the plays in the library.
    expect(within(card).getByText(/plays in your library$/)).toHaveClass("about-card__stats");

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
    await backend.settingsSet({ infoWikipedia: false, infoLastfm: false });
    await loadSettings();
    const a = artistAt(2);
    render(<ArtistPage id={a.id} />);
    await screen.findByRole("heading", { level: 1, name: a.title });
    // No Plex bio, no outside sources: no About section at all.
    expect(screen.queryByRole("button", { name: `About ${a.title}` })).toBeNull();
    expect(screen.queryByText(/listeners on Last\.fm$/)).toBeNull();
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
  it("has the rows under Integrations with Wikipedia on and the keyed sources waiting for keys", async () => {
    render(<SettingsPage />);
    const group = await screen.findByRole("region", { name: "Integrations" });
    expect(within(group).getByRole("heading", { level: 3, name: "Artist info" })).toBeInTheDocument();
    expect(within(group).getByRole("switch", { name: "Wikipedia and Wikidata" })).toBeChecked();
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
    const sw = await within(row("infoListenbrainz")).findByRole("switch", { name: "ListenBrainz (popular songs)" });
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
