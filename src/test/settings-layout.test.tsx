/**
 * The Settings layout: ten groups with a side nav (a dropdown when narrow),
 * the common rows with one More per group (remembered open), integrations
 * set up as cards and the rest under More, tools moved out behind a Manage row, rows that wait for their
 * service, search across all of it, Reset in each group's menu, and the
 * one "Share unreleased music" rule with its exceptions.
 */
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ContextMenuHost } from "../components/ContextMenu";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS, type Settings } from "../lib/types";
import { SettingsPage } from "../pages/SettingsPage";
import { COMMON, GROUPS, MANAGE_IN_MORE, MORE_OPEN_KEY, PAGES, advancedView, inMore, settingsView } from "../pages/settingsMeta";
import { UNRELEASED_OVERRIDES, exceptionsTo, patchForRule } from "../pages/unreleasedRule";
import { useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";

const mock = backend as MockBackend;
const route = () => useRouter.getState().entries[useRouter.getState().index].route;

async function settings(p: Partial<Settings> = {}) {
  await mock.settingsSet({ ...DEFAULT_SETTINGS, ...p });
  await loadSettings();
}

beforeEach(async () => {
  settingsView.inlinePages = false;
  advancedView.startOpen = false;
  localStorage.removeItem(MORE_OPEN_KEY);
  useRouter.getState().reset({ name: "settings" });
  await settings();
});

afterEach(() => {
  settingsView.inlinePages = true;
  advancedView.startOpen = true;
});

const rowKeys = () => [...document.querySelectorAll<HTMLElement>("[data-setting]")].map((r) => r.dataset.setting!);

describe("groups and nav", () => {
  it("shows the ten groups in order, each in the side nav and the narrow dropdown", async () => {
    render(<SettingsPage />);
    await screen.findByRole("region", { name: "Playback" });
    const headings = screen.getAllByRole("heading", { level: 2 }).map((h) => h.textContent);
    expect(headings).toEqual([...GROUPS]);
    const navEl = screen.getByRole("navigation", { name: "Settings sections" });
    expect(within(navEl).getAllByRole("button").map((b) => b.textContent)).toEqual([...GROUPS]);
    const jump = within(navEl).getByRole("combobox", { name: "Go to a section" });
    expect([...(jump as HTMLSelectElement).options].map((o) => o.value)).toEqual([...GROUPS]);
  });

  it("every row is in one of the groups, with no orphans", async () => {
    settingsView.inlinePages = true;
    advancedView.startOpen = true;
    await settings({ discordStatus: true, transcribeLyrics: true, nowPlayingFeed: true });
    render(<SettingsPage />);
    await screen.findByRole("switch", { name: "Private session" });
    for (const el of document.querySelectorAll<HTMLElement>("[data-setting]")) {
      const group = el.closest<HTMLElement>("section[data-group]")?.dataset.group;
      expect(GROUPS as readonly string[], `${el.dataset.setting} is in a group`).toContain(group);
    }
  });

  it("highlights the group in view and the one picked, from the list or the dropdown", async () => {
    let seen: ((entries: { target: Element; isIntersecting: boolean }[]) => void) | null = null;
    class Observer {
      constructor(cb: typeof seen) {
        seen = cb;
      }
      observe() {}
      disconnect() {}
    }
    vi.stubGlobal("IntersectionObserver", Observer);
    const scrolled = vi.fn();
    Element.prototype.scrollIntoView = scrolled;
    try {
      const user = userEvent.setup();
      render(<SettingsPage />);
      const navEl = await screen.findByRole("navigation", { name: "Settings sections" });
      const link = (g: string) => within(navEl).getByRole("button", { name: g });
      expect(link("Account and servers")).toHaveAttribute("aria-current", "true");
      // Scrolling: the first group in view lights up.
      const section = (g: string) => screen.getByRole("region", { name: g });
      act(() =>
        seen!([
          { target: section("Account and servers"), isIntersecting: false },
          { target: section("Sound"), isIntersecting: true },
          { target: section("Library"), isIntersecting: true },
        ]),
      );
      expect(link("Sound")).toHaveAttribute("aria-current", "true");
      expect(link("Account and servers")).not.toHaveAttribute("aria-current");
      // A click jumps there and lights it at once.
      await user.click(link("Lyrics"));
      expect(link("Lyrics")).toHaveAttribute("aria-current", "true");
      expect(scrolled).toHaveBeenCalled();
      expect(screen.getByRole("heading", { level: 2, name: "Lyrics" })).toHaveFocus();
      // The narrow window's dropdown does the same.
      await user.selectOptions(within(navEl).getByRole("combobox", { name: "Go to a section" }), "App");
      expect(link("App")).toHaveAttribute("aria-current", "true");
      expect(within(navEl).getByRole("combobox", { name: "Go to a section" })).toHaveValue("App");
    } finally {
      vi.unstubAllGlobals();
      delete (Element.prototype as { scrollIntoView?: unknown }).scrollIntoView;
    }
  });
});

/** Rows in view in a group, not counting Manage rows for pages. */
const shownRows = (region: HTMLElement) =>
  [...region.querySelectorAll<HTMLElement>("[data-setting]")].filter((r) => !r.dataset.setting!.startsWith("manage-") && !r.closest("[hidden]")).map((r) => r.dataset.setting!);

describe("More", () => {
  it("each group shows its common rows, about a third, with one More for the rest", async () => {
    render(<SettingsPage />);
    await screen.findByRole("region", { name: "Playback" });
    const visible: Record<string, string[]> = {};
    for (const g of GROUPS) {
      const region = screen.getByRole("region", { name: g });
      expect(within(region).queryAllByRole("button", { name: /^More in / }).length, `${g} has at most one More`).toBeLessThanOrEqual(1);
      visible[g] = shownRows(region);
      for (const k of visible[g]) expect(COMMON.has(k), `${k} is common`).toBe(true);
    }
    expect(Object.fromEntries(GROUPS.map((g) => [g, visible[g].length]))).toEqual({
      "Account and servers": 3,
      Playback: 5,
      Sound: 5,
      Library: 6,
      "Downloads and devices": 5,
      Appearance: 5,
      Lyrics: 4,
      Integrations: 0,
      "Sharing and friends": 6,
      App: 5,
    });
  });

  it("the visible share is about a third of every row", async () => {
    advancedView.startOpen = true;
    render(<SettingsPage />);
    await screen.findByRole("region", { name: "Playback" });
    const all = GROUPS.flatMap((g) => shownRows(screen.getByRole("region", { name: g })));
    const common = all.filter((k) => !inMore({ key: k }));
    expect(all.length).toBeGreaterThan(150);
    expect(common.length / all.length).toBeGreaterThan(0.2);
    expect(common.length / all.length).toBeLessThan(0.36);
  });

  it("More opens in place and stays open next time, per section, on this device", async () => {
    const user = userEvent.setup();
    localStorage.removeItem(MORE_OPEN_KEY);
    const first = render(<SettingsPage />);
    const playback = await screen.findByRole("region", { name: "Playback" });
    expect(within(playback).queryByRole("combobox", { name: "Shuffle style" })).toBeNull();
    const more = within(playback).getByRole("button", { name: /^More in Playback, \d+ settings$/ });
    expect(more).toHaveAttribute("aria-expanded", "false");
    await user.click(more);
    expect(more).toHaveAttribute("aria-expanded", "true");
    expect(within(playback).getByRole("combobox", { name: "Shuffle style" })).toBeInTheDocument();
    expect(JSON.parse(localStorage.getItem(MORE_OPEN_KEY) ?? "[]")).toEqual(["Playback"]);
    first.unmount();
    render(<SettingsPage />);
    const again = await screen.findByRole("region", { name: "Playback" });
    expect(within(again).getByRole("button", { name: /^More in Playback/ })).toHaveAttribute("aria-expanded", "true");
    // Other sections stay as they were.
    expect(within(screen.getByRole("region", { name: "Sound" })).getByRole("button", { name: /^More in Sound/ })).toHaveAttribute("aria-expanded", "false");
    localStorage.removeItem(MORE_OPEN_KEY);
  });

  it("search finds a row under More and marks it In More", async () => {
    render(<SettingsPage />);
    const box = await screen.findByRole("searchbox", { name: "Search settings" });
    fireEvent.change(box, { target: { value: "shuffle style" } });
    const row = (await screen.findByRole("combobox", { name: "Shuffle style" })).closest<HTMLElement>(".setting-row")!;
    expect(within(row).getByText("In More")).toBeInTheDocument();
    fireEvent.change(box, { target: { value: "crossfade songs" } });
    const common = (await screen.findByText("Crossfade songs", { selector: ".setting-row__title" })).closest<HTMLElement>(".setting-row")!;
    expect(within(common).queryByText("In More")).toBeNull();
  });
});

describe("pages", () => {
  it("integrations are cards that say whether they're set up and open their page", async () => {
    const user = userEvent.setup();
    await settings({ nowPlayingFeed: true });
    render(<SettingsPage />);
    const group = await screen.findByRole("region", { name: "Integrations" });
    // Only what's set up shows; the rest wait under More.
    const shownCards = () => [...group.querySelectorAll<HTMLElement>(".settings-card")].filter((c) => !c.closest("[hidden]")).map((c) => c.dataset.page);
    expect(shownCards()).toEqual(["widgetFeed"]);
    await user.click(within(group).getByRole("button", { name: /^More in Integrations/ }));
    expect(shownCards()).toEqual(["widgetFeed", ...PAGES.filter((p) => p.kind === "integration" && p.id !== "widgetFeed").map((p) => p.id)]);
    // No integration rows on the main page.
    expect(within(group).queryAllByRole("switch")).toHaveLength(0);
    const feed = group.querySelector<HTMLElement>('[data-page="widgetFeed"]')!;
    expect(feed).toHaveTextContent("Connected");
    const lidarr = group.querySelector<HTMLElement>('[data-page="lidarr"]')!;
    expect(lidarr).toHaveTextContent("Not set up");
    await user.click(within(lidarr).getByRole("button", { name: "Set up Lidarr" }));
    expect(route()).toEqual({ name: "settings", page: "lidarr" });
    const page = await screen.findByRole("region", { name: "Lidarr" });
    expect(within(page).getByRole("textbox", { name: "Lidarr address at home" })).toBeInTheDocument();
    // The nav shows where the page belongs.
    expect(within(screen.getByRole("navigation", { name: "Settings sections" })).getByRole("button", { name: "Integrations" })).toHaveAttribute("aria-current", "true");
    await user.click(within(page).getByRole("button", { name: "Integrations" }));
    expect(route()).toEqual({ name: "settings" });
    expect(await screen.findByRole("region", { name: "Integrations" })).toBeInTheDocument();
  });

  it("tools moved out of a group leave a Manage row that opens their page", async () => {
    const user = userEvent.setup();
    await settings({ transcribeLyrics: true });
    render(<SettingsPage />);
    await screen.findByRole("region", { name: "Library" });
    const pages = PAGES.filter((p) => p.kind === "manage");
    for (const p of pages) {
      const region = screen.getByRole("region", { name: p.group });
      // Rarely needed tools sit under their section's More.
      if (MANAGE_IN_MORE.has(p.id)) {
        const more = within(region).getByRole("button", { name: /^More in / });
        if (more.getAttribute("aria-expanded") === "false") {
          expect(within(region).queryByRole("button", { name: `Manage ${p.title}` })).toBeNull();
          await user.click(more);
        }
      }
      expect(within(region).getByRole("button", { name: `Manage ${p.title}` })).toBeInTheDocument();
      // Their rows aren't on the main page.
      expect(document.querySelector(`[data-page="${p.id}"]`)).toBeNull();
    }
    expect(document.querySelector('[data-setting="musicFolder"]')).toBeNull();
    await user.click(screen.getByRole("button", { name: "Manage Tags and archive" }));
    expect(route()).toEqual({ name: "settings", page: "tags" });
    const page = await screen.findByRole("region", { name: "Tags and archive" });
    expect(within(page).getByText("Music folder")).toBeInTheDocument();
    expect(within(page).getByRole("switch", { name: "Show lineage" })).toBeInTheDocument();
    for (const p of pages) {
      act(() => useRouter.getState().navigate({ name: "settings", page: p.id }));
      expect(await screen.findByRole("region", { name: p.title })).toBeInTheDocument();
    }
  });

  it("a link to a row opens its page, with its More open", async () => {
    act(() => useRouter.getState().navigate({ name: "settings", row: "showSoundMap" }));
    const { unmount } = render(<SettingsPage />);
    const library = await screen.findByRole("region", { name: "Library" });
    expect(within(library).getByRole("switch", { name: "Show the sound map" })).toBeInTheDocument();
    unmount();
    act(() => useRouter.getState().navigate({ name: "settings", row: "releaseStatusTag" }));
    render(<SettingsPage />);
    const tags = await screen.findByRole("region", { name: "Tags and archive" });
    expect(tags.querySelector('[data-setting="releaseStatusTag"]')).not.toBeNull();
  });

  it("Lyrics transcription has a Manage row only while transcription is on", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    const lyrics = await screen.findByRole("region", { name: "Lyrics" });
    expect(within(lyrics).queryByRole("button", { name: "Manage Lyrics transcription" })).toBeNull();
    await user.click(within(lyrics).getByRole("switch", { name: "Transcribe songs without lyrics on this computer" }));
    expect(await within(lyrics).findByRole("button", { name: "Manage Lyrics transcription" })).toBeInTheDocument();
  });

  it("a page's rows wait until its service is connected", async () => {
    const user = userEvent.setup();
    render(<SettingsPage page="discord" />);
    const page = await screen.findByRole("region", { name: "Discord" });
    expect(rowKeys()).toEqual(["discordStatus"]);
    expect(within(page).getByRole("note")).toHaveTextContent("More settings show once this is connected.");
    await user.click(within(page).getByRole("switch", { name: "Show what you're playing on Discord" }));
    await waitFor(() => expect(rowKeys()).toContain("discordCoverSource"));
    expect(rowKeys()).toContain("discordFindSong");
    expect(within(page).queryByRole("note")).toBeNull();
    // The finer ones fold under the page's More.
    await user.click(within(page).getByRole("button", { name: /^More in Discord/ }));
    expect(rowKeys()).toEqual(expect.arrayContaining(["discordStatusLine", "discordListenAlong", "discordClearOnPause"]));
  });

  it("the widget feed's port and Discogs import wait for theirs too", async () => {
    advancedView.startOpen = true;
    const { unmount } = render(<SettingsPage page="widgetFeed" />);
    await screen.findByRole("region", { name: "Desktop widget feed" });
    expect(rowKeys()).toEqual(["nowPlayingFeed"]);
    unmount();
    await settings({ nowPlayingFeed: true });
    const again = render(<SettingsPage page="widgetFeed" />);
    await waitFor(() => expect(rowKeys()).toEqual(["nowPlayingFeed", "nowPlayingFeedPort"]));
    again.unmount();
    render(<SettingsPage page="discogs" />);
    await screen.findByRole("region", { name: "Discogs" });
    expect(rowKeys()).toEqual(["discogsToken"]);
  });
});

describe("search", () => {
  it("reaches every row, on the main page or any page, those under More included", async () => {
    // Every row as the test setup shows them: pages in place, services connected.
    settingsView.inlinePages = true;
    advancedView.startOpen = true;
    await settings({ discordStatus: true, transcribeLyrics: true, nowPlayingFeed: true });
    const all = render(<SettingsPage />);
    await screen.findByRole("switch", { name: "Private session" });
    const rows = [...document.querySelectorAll<HTMLElement>("[data-setting]")]
      .filter((r) => !r.dataset.setting!.startsWith("manage-"))
      .map((r) => ({ key: r.dataset.setting!, title: r.querySelector(".setting-row__title")!.textContent!.trim() }));
    expect(rows.length).toBeGreaterThan(150);
    all.unmount();

    settingsView.inlinePages = false;
    advancedView.startOpen = false;
    render(<SettingsPage />);
    const box = await screen.findByRole("searchbox", { name: "Search settings" });
    const missed: string[] = [];
    for (const r of rows) {
      fireEvent.change(box, { target: { value: r.title } });
      // Sections that load their own state (the Stream Deck plugin) show a moment later.
      await waitFor(() => document.querySelector(`[data-setting="${r.key}"]`) ?? Promise.reject()).catch(() => missed.push(`${r.key} (${r.title})`));
    }
    expect(missed).toEqual([]);
  });

  it("finds words in a row's longer text and offers a page whose rows are waiting", async () => {
    render(<SettingsPage />);
    const box = await screen.findByRole("searchbox", { name: "Search settings" });
    // "Litterbox" is only in the cover art row's longer text, and Discord isn't on.
    fireEvent.change(box, { target: { value: "litterbox" } });
    const discord = await screen.findByRole("button", { name: "Manage Discord" });
    expect(discord.closest("section")?.querySelector("h2")).toHaveTextContent("Integrations");
    fireEvent.change(box, { target: { value: "ntfy" } });
    expect(await screen.findByRole("textbox", { name: "Relay for listening sessions" })).toBeInTheDocument();
    // A row on a page shows under its page's name, with a way there.
    fireEvent.change(box, { target: { value: "release status values" } });
    expect(await screen.findByRole("heading", { level: 3, name: "Tags and archive" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Open Tags and archive" }));
    expect(route()).toEqual({ name: "settings", page: "tags" });
  });

  it("a row's longer text opens behind its ?", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    const more = await screen.findByRole("button", { name: "More about Crossfade songs" });
    const pop = document.getElementById(more.getAttribute("aria-controls")!)!;
    expect(pop).not.toBeVisible();
    await user.click(more);
    expect(more).toHaveAttribute("aria-expanded", "true");
    expect(pop).toBeVisible();
    expect(pop).toHaveTextContent("Skipping always cuts straight to the next song.");
    await user.keyboard("{Escape}");
    expect(pop).not.toBeVisible();
  });
});

describe("Reset this section", () => {
  it("is in each group's ⋯ menu and on a page's, and resets only that", async () => {
    const user = userEvent.setup();
    await settings({ leveling: "off", shuffleStyle: "standard", discordStatus: true, discordClearOnPause: false });
    render(
      <>
        <SettingsPage />
        <ContextMenuHost />
      </>,
    );
    await user.click(await screen.findByRole("button", { name: "More options for Sound" }));
    await user.click(await screen.findByRole("menuitem", { name: "Reset this section" }));
    await user.click(within(screen.getByRole("dialog", { name: "Reset Sound settings?" })).getByRole("button", { name: "Reset" }));
    await waitFor(() => expect(useSettings.getState().settings?.leveling).toBe(DEFAULT_SETTINGS.leveling));
    expect(useSettings.getState().settings?.shuffleStyle).toBe("standard");
    // A page's menu resets its own rows.
    act(() => useRouter.getState().navigate({ name: "settings", page: "discord" }));
    await user.click(await screen.findByRole("button", { name: "More options for Discord" }));
    await user.click(await screen.findByRole("menuitem", { name: "Reset this section" }));
    await user.click(within(screen.getByRole("dialog", { name: "Reset Discord settings?" })).getByRole("button", { name: "Reset" }));
    await waitFor(() => expect(useSettings.getState().settings?.discordClearOnPause).toBe(DEFAULT_SETTINGS.discordClearOnPause));
    expect(useSettings.getState().settings?.shuffleStyle).toBe("standard");
  });
});

describe("Share unreleased music", () => {
  it("starts at Ask each time, and picking a rule sets every feature's switch", () => {
    expect(DEFAULT_SETTINGS.unreleasedSharing).toBe("ask");
    expect(exceptionsTo(DEFAULT_SETTINGS)).toEqual([]);
    const never = patchForRule("never");
    for (const o of UNRELEASED_OVERRIDES) expect(never[o.key], o.key).toBe(!o.onShares);
    const always = patchForRule("always");
    for (const o of UNRELEASED_OVERRIDES) expect(always[o.key], o.key).toBe(o.onShares);
    expect(always.listenHideUnreleased).toBe(false);
    const ask = patchForRule("ask");
    for (const o of UNRELEASED_OVERRIDES) expect(ask[o.key], o.key).toBe(DEFAULT_SETTINGS[o.key]);
    // A switch that differs from the rule is an exception.
    expect(exceptionsTo({ ...DEFAULT_SETTINGS, ...never, mixUnreleased: true })).toEqual(["mixUnreleased"]);
  });

  it("is one row in Sharing and friends, under More, with the per-feature choices", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    const sharing = await screen.findByRole("region", { name: "Sharing and friends" });
    expect(within(sharing).queryByRole("combobox", { name: "Share unreleased music" })).toBeNull();
    await user.click(within(sharing).getByRole("button", { name: /^More in Sharing/ }));
    const rule = within(sharing).getByRole("combobox", { name: "Share unreleased music" });
    expect(rule).toHaveValue("ask");
    await user.selectOptions(rule, "never");
    await waitFor(() => expect(useSettings.getState().settings?.unreleasedSharing).toBe("never"));
    const s = (await mock.settingsGet()) as Settings;
    expect(s).toMatchObject({ mixUnreleased: false, discordShowUnreleased: false, discordUnreleasedArt: false, listenHideUnreleased: true, shareUnreleased: false });
    expect(within(sharing).getByRole("heading", { level: 3, name: "Unreleased music, feature by feature" })).toBeInTheDocument();
    const mixes = within(sharing).getByRole("switch", { name: "Include unreleased music in mixes" });
    expect(mixes).not.toBeChecked();
    await user.click(mixes);
    expect(await within(sharing).findByText("1 feature is set differently under More.")).toBeInTheDocument();
    await user.selectOptions(rule, "always");
    await waitFor(() => expect(useSettings.getState().settings).toMatchObject({ unreleasedSharing: "always", scrobbleUnreleased: true, shareTranscriptionsUnreleased: true, listenHideUnreleased: false }));
    expect(within(sharing).queryByText(/set differently/)).toBeNull();
  });

  it("Ask each time never turns Discord sharing on, and asks before undoing exceptions", async () => {
    const never = { ...DEFAULT_SETTINGS, ...patchForRule("never") } as Settings;
    const ask = patchForRule("ask", never);
    // Discord can't ask, so it stays off; nothing else that can't ask starts sharing either.
    expect(ask).toMatchObject({ discordShowUnreleased: false, discordUnreleasedArt: false, mixUnreleased: false, listenHideUnreleased: true });
    for (const o of UNRELEASED_OVERRIDES) expect(ask[o.key] === o.onShares && never[o.key] !== o.onShares, o.key).toBe(false);
    // Turned on by the user, it stays on.
    expect(patchForRule("ask", { ...never, discordShowUnreleased: true }).discordShowUnreleased).toBe(true);

    const user = userEvent.setup();
    render(<SettingsPage />);
    const sharing = await screen.findByRole("region", { name: "Sharing and friends" });
    await user.click(within(sharing).getByRole("button", { name: /^More in Sharing/ }));
    const rule = within(sharing).getByRole("combobox", { name: "Share unreleased music" });
    await user.selectOptions(rule, "never");
    await waitFor(() => expect(useSettings.getState().settings?.unreleasedSharing).toBe("never"));
    await user.selectOptions(rule, "ask");
    await waitFor(() => expect(useSettings.getState().settings?.unreleasedSharing).toBe("ask"));
    expect(useSettings.getState().settings).toMatchObject({ discordShowUnreleased: false, discordUnreleasedArt: false });
    // An exception the next rule would undo: asked first, and Cancel keeps it.
    await user.selectOptions(rule, "never");
    await waitFor(() => expect(useSettings.getState().settings?.unreleasedSharing).toBe("never"));
    await user.click(within(sharing).getByRole("switch", { name: "Scrobble unreleased music" }));
    await waitFor(() => expect(useSettings.getState().settings?.scrobbleUnreleased).toBe(true));
    await user.selectOptions(rule, "ask");
    const confirm = await screen.findByRole("dialog", { name: "Undo your exceptions?" });
    await user.click(within(confirm).getByRole("button", { name: "Cancel" }));
    expect(useSettings.getState().settings).toMatchObject({ unreleasedSharing: "never", scrobbleUnreleased: true });
  });
});
