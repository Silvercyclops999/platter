/**
 * "Same artist under two names" in Settings: joining two played artists
 * makes them one in Stats (their plays added up under the name played
 * most), each merge reads as a sentence, and Undo splits them again.
 */
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { backend } from "../lib/backend";
import { SettingsPage } from "../pages/SettingsPage";
import { ArtistAliases } from "../pages/ArtistAliases";

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

  it("has a row in Settings under Library", async () => {
    render(<SettingsPage />);
    const row = document.querySelector('[data-setting="artistAliases"]') as HTMLElement;
    expect(within(row).getByText("Same artist under two names")).toBeInTheDocument();
    expect(within(row).getByRole("button", { name: "Count as one" })).toBeDisabled();
  });
});
