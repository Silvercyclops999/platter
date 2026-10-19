/**
 * "Same artist under two names" in Settings: joining two played artists
 * makes them one in Stats (their plays added up under the name played
 * most), each merge reads as a sentence, and Undo splits them again.
 */
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { Toaster } from "../components/Toast";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS } from "../lib/types";
import { SettingsPage } from "../pages/SettingsPage";
import { ArtistAliases } from "../pages/ArtistAliases";
import { loadSettings } from "../stores/settings";

const mock = backend as MockBackend;

const ids = { title: "t", desc: "d" };

describe("artists counted as one", () => {
  it("joins two names, adds up their plays under the one played most, and undoes", async () => {
    const user = userEvent.setup();
    const before = await backend.statsOverview({ from: null, to: null }, 100);
    const [a, b] = before.topArtists;
    render(<ArtistAliases ids={ids} />);
    await waitFor(async () => expect((await backend.statsPlayedArtists("", 100)).length).toBeGreaterThan(1));
    await user.type(screen.getByRole("combobox", { name: "Artist" }), b.name);
    await user.type(screen.getByRole("combobox", { name: "Same artist as" }), a.name);
    await user.click(screen.getByRole("button", { name: "Count as one" }));

    const list = await screen.findByRole("list", { name: "Artists counted as one" });
    expect(within(list).getByRole("listitem")).toHaveTextContent(`${b.name} counts as ${a.name}`);
    const after = await backend.statsOverview({ from: null, to: null }, 100);
    expect(after.topArtists[0]).toMatchObject({ name: a.name, plays: a.plays + b.plays });
    expect(after.topArtists.some((x) => x.name === b.name)).toBe(false);
    expect(after.totals.artists).toBe(before.totals.artists - 1);

    fireEvent.click(screen.getByRole("button", { name: `Count ${b.name} on its own again` }));
    await waitFor(() => expect(screen.queryByRole("list", { name: "Artists counted as one" })).toBeNull());
    expect((await backend.statsOverview({ from: null, to: null }, 100)).topArtists.slice(0, 2).map((x) => x.name)).toEqual([a.name, b.name]);
  });

  it("says so when a typed name isn't an artist you've played", async () => {
    const user = userEvent.setup();
    const set = vi.spyOn(backend, "artistAliasSet");
    render(
      <>
        <ArtistAliases ids={ids} />
        <Toaster />
      </>,
    );
    await user.type(screen.getByRole("combobox", { name: "Artist" }), "Nobody Ever");
    await user.type(screen.getByRole("combobox", { name: "Same artist as" }), "Also Nobody");
    await user.click(screen.getByRole("button", { name: "Count as one" }));
    expect((await screen.findAllByText("No artist you've played is called Nobody Ever."))[0]).toBeInTheDocument();
    expect(set).not.toHaveBeenCalled();
  });

  it("lists names matched to one artist, keeps a pair apart, and joins them again", async () => {
    const user = userEvent.setup();
    mock.stats.autoPairs = [{ key: "northwind", name: "Northwind", sameAs: "the northwinds", sameAsName: "The Northwinds" }];
    render(<ArtistAliases ids={ids} />);
    const matched = await screen.findByRole("list", { name: "Matched to one artist" });
    expect(within(matched).getByRole("listitem")).toHaveTextContent("Northwind and The Northwinds are matched to one artist");
    await user.click(screen.getByRole("button", { name: "Northwind and The Northwinds are not the same artist" }));
    const apart = await screen.findByRole("list", { name: "Kept apart" });
    expect(within(apart).getByRole("listitem")).toHaveTextContent("Northwind and The Northwinds count apart");
    await waitFor(() => expect(screen.queryByRole("list", { name: "Matched to one artist" })).toBeNull());
    await user.click(screen.getByRole("button", { name: "Count Northwind and The Northwinds as one again" }));
    await screen.findByRole("list", { name: "Matched to one artist" });
    expect(screen.queryByRole("list", { name: "Kept apart" })).toBeNull();
    mock.stats.autoPairs = [];
  });

  it("the row's switch turns off joining by MusicBrainz id, on by default", async () => {
    const user = userEvent.setup();
    await backend.settingsSet({ ...DEFAULT_SETTINGS });
    await loadSettings();
    mock.stats.autoPairs = [{ key: "northwind", name: "Northwind", sameAs: "the northwinds", sameAsName: "The Northwinds" }];
    render(<SettingsPage />);
    const row = () => document.querySelector('[data-setting="artistAliases"]') as HTMLElement;
    const sw = await waitFor(() => within(row()).getByRole("switch"));
    expect(sw).toHaveAttribute("aria-checked", "true");
    await within(row()).findByRole("list", { name: "Matched to one artist" });
    await user.click(sw);
    await waitFor(async () => expect((await backend.settingsGet()).artistAliasesAuto).toBe(false));
    await waitFor(() => expect(within(row()).queryByRole("list", { name: "Matched to one artist" })).toBeNull());
    mock.stats.autoPairs = [];
  });

  it("has a row in Settings under Library", async () => {
    render(<SettingsPage />);
    const row = document.querySelector('[data-setting="artistAliases"]') as HTMLElement;
    expect(within(row).getByText("Same artist under two names")).toBeInTheDocument();
    expect(within(row).getByRole("button", { name: "Count as one" })).toBeDisabled();
  });
});
