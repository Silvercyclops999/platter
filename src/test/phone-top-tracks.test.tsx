/**
 * The phone artist page's top tracks: All time ranks by everyone's plays,
 * Yours by your own, the pick is the shared Popular setting, and a save
 * that fails puts the pill back.
 */
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS, type Settings } from "../lib/types";
import { PhoneArtist, rankPatch, topRank, TOP_TRACKS } from "../phone/pages/Artist";
import { usePhoneForm } from "../phone/form";
import { loadSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";

const mock = backend as MockBackend;
const set = (patch: Partial<Settings>): Settings => ({ ...DEFAULT_SETTINGS, ...patch });

async function useSettingsOf(patch: Partial<Settings>) {
  await mock.settingsSet(set(patch));
  await loadSettings();
}

/** Titles of the page's top tracks under a ranking, as the core gives them. */
async function topTitles(id: string, patch: Partial<Settings>): Promise<string[]> {
  await useSettingsOf(patch);
  return (await mock.artistPage(id)).popular.slice(0, TOP_TRACKS).map((t) => t.title);
}

/** An artist whose top five differ between everyone's plays and yours. */
async function artistWithTwoOrders() {
  for (const a of mock.fx.artists) {
    const all = await topTitles(a.id, { infoListenbrainz: true, popularSource: "global" });
    const yours = await topTitles(a.id, { infoListenbrainz: true, popularSource: "myPlays" });
    if (all.length >= 3 && all.join() !== yours.join()) return { id: a.id, all, yours };
  }
  throw new Error("no fixture artist has two orders");
}

const shownTitles = (list: HTMLElement) => within(list).getAllByRole("listitem").map((li) => li.querySelector(".ph-row__title")?.textContent ?? "");

beforeEach(async () => {
  usePhoneForm.setState({ phone: true });
  await useSettingsOf({});
});

afterEach(() => {
  usePhoneForm.setState({ phone: false });
  vi.restoreAllMocks();
});

describe("which ranking is on", () => {
  it("is All time only with the setting on everyone's plays and a source for it", () => {
    expect(topRank(set({ popularSource: "global", infoListenbrainz: true }))).toBe("all");
    expect(topRank(set({ popularSource: "global", infoLastfm: true }))).toBe("all");
    expect(topRank(set({ popularSource: "global" }))).toBe("yours");
    expect(topRank(set({ popularSource: "myPlays", infoListenbrainz: true }))).toBe("yours");
    expect(topRank(null)).toBe("yours");
  });

  it("All time turns ListenBrainz on only when neither source is on", () => {
    expect(rankPatch("all", set({}))).toEqual({ popularSource: "global", infoListenbrainz: true });
    expect(rankPatch("all", set({ infoLastfm: true }))).toEqual({ popularSource: "global" });
    expect(rankPatch("yours", set({ infoListenbrainz: true }))).toEqual({ popularSource: "myPlays" });
  });
});

describe("the pills on the artist page", () => {
  it("switch the list between everyone's order and yours, and save the choice", async () => {
    const { id, all, yours } = await artistWithTwoOrders();
    await useSettingsOf({ infoListenbrainz: true, popularSource: "global" });
    const info = vi.spyOn(mock, "artistInfo");
    render(<PhoneArtist id={id} />);
    const section = await screen.findByRole("region", { name: "Top tracks" });
    const pills = within(section).getByRole("group", { name: "Rank top tracks by" });
    expect(within(pills).getByRole("button", { name: "All time" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.queryByText("By your plays")).toBeNull();
    const list = section.querySelector("ul")!;
    await waitFor(() => expect(shownTitles(list).slice(0, all.length)).toEqual(all));
    // Everyone's list is asked for, so a first visit fetches it.
    expect(info).toHaveBeenCalledWith(id);

    fireEvent.click(within(pills).getByRole("button", { name: "Yours" }));
    expect(within(pills).getByRole("button", { name: "Yours" })).toHaveAttribute("aria-pressed", "true");
    await waitFor(() => expect(shownTitles(list).slice(0, yours.length)).toEqual(yours));
    expect((await mock.settingsGet()).popularSource).toBe("myPlays");

    fireEvent.click(within(pills).getByRole("button", { name: "All time" }));
    await waitFor(() => expect(shownTitles(list).slice(0, all.length)).toEqual(all));
    expect((await mock.settingsGet()).popularSource).toBe("global");
  });

  it("asks for the list again only once a slow save has landed", async () => {
    const { id, all, yours } = await artistWithTwoOrders();
    await useSettingsOf({ infoListenbrainz: true, popularSource: "myPlays" });
    render(<PhoneArtist id={id} />);
    const section = await screen.findByRole("region", { name: "Top tracks" });
    const list = section.querySelector("ul")!;
    await waitFor(() => expect(shownTitles(list).slice(0, yours.length)).toEqual(yours));
    const save = mock.settingsSet.bind(mock);
    vi.spyOn(mock, "settingsSet").mockImplementation((patch) => new Promise((ok) => setTimeout(ok, 300)).then(() => save(patch)));
    fireEvent.click(within(section).getByRole("button", { name: "All time" }));
    await waitFor(() => expect(shownTitles(list).slice(0, all.length)).toEqual(all), { timeout: 3000 });
  });

  it("All time with no source on turns ListenBrainz on", async () => {
    const { id } = await artistWithTwoOrders();
    await useSettingsOf({ popularSource: "global" });
    render(<PhoneArtist id={id} />);
    const pills = await screen.findByRole("group", { name: "Rank top tracks by" });
    expect(within(pills).getByRole("button", { name: "Yours" })).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(within(pills).getByRole("button", { name: "All time" }));
    await waitFor(() => expect(within(pills).getByRole("button", { name: "All time" })).toHaveAttribute("aria-pressed", "true"));
    const saved = await mock.settingsGet();
    expect([saved.popularSource, saved.infoListenbrainz]).toEqual(["global", true]);
  });

  it("puts the pill back and says so when the choice can't be saved", async () => {
    const { id, yours } = await artistWithTwoOrders();
    await useSettingsOf({ infoListenbrainz: true, popularSource: "myPlays" });
    render(<PhoneArtist id={id} />);
    const section = await screen.findByRole("region", { name: "Top tracks" });
    const pills = within(section).getByRole("group", { name: "Rank top tracks by" });
    vi.spyOn(mock, "settingsSet").mockRejectedValue(new Error("disk full"));
    await act(async () => {
      fireEvent.click(within(pills).getByRole("button", { name: "All time" }));
    });
    await waitFor(() => expect(within(pills).getByRole("button", { name: "Yours" })).toHaveAttribute("aria-pressed", "true"));
    expect(useSettings.getState().settings?.popularSource).toBe("myPlays");
    expect(useToasts.getState().toasts.map((t) => t.message).join()).toContain("Couldn't save that setting");
    expect(shownTitles(section.querySelector("ul")!).slice(0, yours.length)).toEqual(yours);
  });
});
