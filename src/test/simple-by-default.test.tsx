/**
 * Simple by default: the profile menu keeps seven doors, and everything that
 * left it is reachable where it belongs now. Create playlist is the + in Your
 * Library, Recents is on Home, the diary and Wrapped are tabs in Stats, Crates
 * and the sound map are in Home's Explore (and can be pinned to Your Library),
 * the miniplayer is the player bar's button, keyboard shortcuts open from Help
 * and still from their keys, and Go offline is in Settings and the tray.
 * The phone's Home keeps to five sections.
 */
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { ContextMenuHost } from "../components/ContextMenu";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS } from "../lib/types";
import { HomeExplore } from "../pages/HomeExplore";
import { HomePage } from "../pages/HomePage";
import { StatsPage } from "../pages/StatsPage";
import { COMMON, MORE_OPEN_KEY } from "../pages/settingsMeta";
import { PhoneShell } from "../phone/PhoneShell";
import { usePhoneForm } from "../phone/form";
import { MENU_ACTIONS } from "../shell/menuBar";
import { PlayerBar } from "../shell/PlayerBar";
import { ShortcutsHelp } from "../shell/ShortcutsHelp";
import { Sidebar } from "../shell/Sidebar";
import { useShortcuts } from "../shell/useShortcuts";
import { HelpPanel } from "../help/HelpPanel";
import { usePinnedPages } from "../stores/pinnedPages";
import { useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import { useLikes } from "../stores/likes";
import { useUi } from "../stores/ui";

const mock = backend as MockBackend;
const route = () => useRouter.getState().entries[useRouter.getState().index].route;
const SLOW = { timeout: 5000 };

beforeEach(async () => {
  await mock.settingsSet({ ...DEFAULT_SETTINGS });
  useSettings.setState({ settings: null });
  await loadSettings();
  useRouter.getState().reset();
  useUi.setState({ shortcutsOpen: false });
  localStorage.removeItem("platter.pinnedPages");
  usePinnedPages.setState({ pages: [] });
});

afterEach(() => {
  usePhoneForm.setState({ phone: false });
});

describe("what left the profile menu", () => {
  it("Create playlist is the + in Your Library", async () => {
    const user = userEvent.setup();
    render(
      <>
        <Sidebar collapsed={false} />
        <ContextMenuHost />
      </>,
    );
    await user.click(screen.getByRole("button", { name: "Create playlist or folder" }));
    expect(await screen.findByRole("menuitem", { name: /^Playlist/ })).toBeInTheDocument();
  });

  it("Recents is Home's Recently played Show all", async () => {
    const user = userEvent.setup();
    render(<HomePage />);
    const title = await screen.findByRole("heading", { name: "Recently played" }, SLOW);
    await user.click(within(title.closest("section")!).getByRole("button", { name: "Show all" }));
    expect(route()).toEqual({ name: "recents" });
  });

  it("the listening diary and Wrapped are tabs in Stats", async () => {
    const user = userEvent.setup();
    render(<StatsPage />);
    const tabs = await screen.findByRole("tablist", { name: "Stats" }, SLOW);
    expect(within(tabs).getAllByRole("tab").map((t) => t.textContent)).toEqual(["Overview", "Recaps", "Wrapped", "Diary", "Compare"]);
    await user.click(within(tabs).getByRole("tab", { name: "Wrapped" }));
    // This year's Wrapped is there any time, as the menu's was.
    expect(await screen.findByRole("button", { name: `${new Date().getFullYear()} Wrapped` }, SLOW)).toBeInTheDocument();
    await user.click(within(tabs).getByRole("tab", { name: "Diary" }));
    expect(await screen.findByRole("heading", { level: 2, name: "Listening diary" }, SLOW)).toBeInTheDocument();
  });

  it("the Diary tab goes while the diary isn't kept", async () => {
    await mock.settingsSet({ ...DEFAULT_SETTINGS, keepDiary: false });
    await loadSettings();
    render(<StatsPage />);
    const tabs = await screen.findByRole("tablist", { name: "Stats" }, SLOW);
    expect(within(tabs).queryByRole("tab", { name: "Diary" })).toBeNull();
  });

  it("Crates and the sound map are in Home's Explore, and pin to Your Library", async () => {
    const user = userEvent.setup();
    await mock.settingsSet({ ...DEFAULT_SETTINGS, automix: true });
    await loadSettings();
    const explore = render(<HomeExplore />);
    const region = await screen.findByRole("region", { name: "Explore" });
    await user.click(within(region).getByRole("button", { name: /^Crates/ }));
    expect(route()).toEqual({ name: "crates" });
    await user.click(await within(region).findByRole("button", { name: /^Sound map/ }, SLOW));
    expect(route()).toEqual({ name: "soundMap" });
    await user.click(within(region).getByRole("button", { name: "Pin Crates to Your Library" }));
    expect(within(region).getByRole("button", { name: "Unpin Crates from Your Library" })).toBeInTheDocument();
    expect(JSON.parse(localStorage.getItem("platter.pinnedPages") ?? "[]")).toEqual(["crates"]);
    explore.unmount();
    render(<Sidebar collapsed={false} />);
    const pinned = await screen.findByText("Crates", {}, SLOW);
    expect(pinned.closest("[data-entry], li, [role='row'], [role='option']") ?? pinned).toBeInTheDocument();
    fireEvent.click(pinned);
    await waitFor(() => expect(route()).toEqual({ name: "crates" }));
  });

  it("the miniplayer is the player bar's button", async () => {
    render(<PlayerBar />);
    expect(await screen.findByRole("button", { name: "Open miniplayer" }, SLOW)).toBeInTheDocument();
  });

  it("keyboard shortcuts open from Help, and Ctrl+/ still opens them", async () => {
    const user = userEvent.setup();
    function Keys() {
      useShortcuts();
      return null;
    }
    render(
      <>
        <Keys />
        <HelpPanel />
        <ShortcutsHelp />
      </>,
    );
    await user.click(screen.getByRole("button", { name: /^Keyboard shortcuts/ }));
    expect(await screen.findByRole("dialog", { name: "Keyboard shortcuts" })).toBeInTheDocument();
    act(() => useUi.setState({ shortcutsOpen: false }));
    fireEvent.keyDown(window, { key: "/", code: "Slash", ctrlKey: true });
    expect(await screen.findByRole("dialog", { name: "Keyboard shortcuts" })).toBeInTheDocument();
    // Ctrl+, still opens Settings.
    act(() => useUi.setState({ shortcutsOpen: false }));
    fireEvent.keyDown(window, { key: ",", code: "Comma", ctrlKey: true });
    await waitFor(() => expect(route()).toEqual({ name: "settings" }));
  });

  it("Go offline is the tray's (and menu bar's) action, the same as Settings' switch", async () => {
    act(() => MENU_ACTIONS.goOffline.run());
    await waitFor(() => expect(useSettings.getState().settings?.goOffline).toBe(true));
    act(() => MENU_ACTIONS.goOffline.run());
    await waitFor(() => expect(useSettings.getState().settings?.goOffline).toBe(false));
    // Settings keeps its switch in view (not under More).
    expect(COMMON.has("goOffline")).toBe(true);
  });
});

describe("the phone's Home", () => {
  it("keeps to its sections: on the platter, spun this week, your mixes, two shelves and Explore", async () => {
    usePhoneForm.setState({ phone: true });
    localStorage.removeItem(MORE_OPEN_KEY);
    render(<PhoneShell />);
    await screen.findByRole("list", { name: "Spun this week" }, SLOW);
    await screen.findByRole("region", { name: "Explore" }, SLOW);
    await screen.findByText("Your mixes", {}, SLOW);
    const home = document.querySelector<HTMLElement>(".ph-home")!;
    expect(within(home).getByTestId("on-the-platter")).toBeInTheDocument();
    const titles = [...home.querySelectorAll(".ph-shelf__title, h2:not(.ph-platter__title)")].map((h) => h.textContent?.trim()).filter(Boolean);
    expect(new Set(titles)).toEqual(new Set(["Spun this week", "Your mixes", "Recently added", "Your top artists", "Explore"]));
    // No type pills: Library has them.
    expect(within(home).queryByRole("group", { name: "Show" })).toBeNull();
    // No tools or settings on Home.
    expect(within(home).queryByRole("button", { name: /Settings|Library doctor|Downloads/ })).toBeNull();
  });

  it("leads Spun this week with Liked Songs, as the computer does, in two rows of three", async () => {
    usePhoneForm.setState({ phone: true });
    useLikes.getState().setIds(["t1"]);
    render(<PhoneShell />);
    const grid = await screen.findByRole("list", { name: "Spun this week" }, SLOW);
    await waitFor(() => expect(within(grid).getAllByRole("listitem")[0]).toHaveTextContent("Liked Songs"), SLOW);
    expect(within(grid).getAllByRole("listitem").length).toBeLessThanOrEqual(6);
  });
});
