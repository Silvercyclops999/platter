import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { artUrl, backend } from "../lib/backend";
import { DEFAULT_SETTINGS } from "../lib/types";
import { Sidebar } from "../shell/Sidebar";
import { TOPBAR_COLLAPSE_WIDTH, TopBar } from "../shell/TopBar";
import { useApp } from "../stores/app";
import { nav, useRouter } from "../stores/router";
import { useSettings } from "../stores/settings";
import { useUi } from "../stores/ui";

const route = () => useRouter.getState().entries[useRouter.getState().index].route;
const startWidth = window.innerWidth;

function setWidth(w: number) {
  act(() => {
    Object.defineProperty(window, "innerWidth", { configurable: true, value: w });
    window.dispatchEvent(new Event("resize"));
  });
}

beforeEach(() => {
  useRouter.getState().reset({ name: "home" });
  useSettings.setState({ settings: { ...DEFAULT_SETTINGS } });
  useUi.setState({ paletteOpen: false, searchFocusTick: 0, menu: null });
  useApp.setState({ auth: { state: "signedIn", username: "listener", needsServerPick: false } });
  setWidth(1280);
});
afterEach(() => {
  vi.restoreAllMocks();
  setWidth(startWidth);
});

describe("back and forward", () => {
  it("are greyed out with no history and follow it", async () => {
    render(<TopBar />);
    const back = screen.getByRole("button", { name: "Go back" });
    const forward = screen.getByRole("button", { name: "Go forward" });
    expect(back).toBeDisabled();
    expect(forward).toBeDisabled();

    act(() => nav.go({ name: "settings" }));
    expect(back).toBeEnabled();
    expect(forward).toBeDisabled();

    fireEvent.click(back);
    expect(route()).toEqual({ name: "home" });
    expect(back).toBeDisabled();
    expect(forward).toBeEnabled();
    fireEvent.click(forward);
    expect(route()).toEqual({ name: "settings" });
  });

  it("Home goes home and shows when you're there", () => {
    useRouter.getState().reset({ name: "settings" });
    render(<TopBar />);
    const home = screen.getByRole("button", { name: "Home" });
    expect(home).not.toHaveAttribute("aria-current");
    fireEvent.click(home);
    expect(route()).toEqual({ name: "home" });
    expect(home).toHaveAttribute("aria-current", "page");
  });
});

describe("narrow windows", () => {
  it("collapse the pill to a round search button beside Home", async () => {
    const user = userEvent.setup();
    render(<TopBar />);
    expect(screen.getByRole("combobox", { name: "Search" })).toBeInTheDocument();

    setWidth(TOPBAR_COLLAPSE_WIDTH - 60);
    expect(screen.queryByRole("combobox", { name: "Search" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Home" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /^App menu/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Go back" })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Search" }));
    await waitFor(() => expect(screen.getByRole("combobox", { name: "Search" })).toHaveFocus());
    // Escape on an empty box folds it back into the button.
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("combobox", { name: "Search" })).not.toBeInTheDocument();

    setWidth(TOPBAR_COLLAPSE_WIDTH + 100);
    expect(screen.getByRole("combobox", { name: "Search" })).toBeInTheDocument();
  });

  it("Ctrl+L opens the collapsed pill and focuses it", async () => {
    setWidth(800);
    render(<TopBar />);
    act(() => useUi.getState().focusSearch());
    await waitFor(() => expect(screen.getByRole("combobox", { name: "Search" })).toHaveFocus());
  });
});

describe("app menu", () => {
  it("opens from the avatar with Settings, Private session, Check for updates and Sign out", async () => {
    const user = userEvent.setup();
    render(<TopBar />);
    const avatar = screen.getByRole("button", { name: "App menu, signed in as listener" });
    expect(avatar).toHaveAttribute("aria-expanded", "false");
    await user.click(avatar);
    const menu = screen.getByRole("menu", { name: "App menu" });
    expect(avatar).toHaveAttribute("aria-expanded", "true");
    const items = [...menu.querySelectorAll('[role^="menuitem"]')].map((i) => i.textContent);
    expect(items).toEqual(["Create playlist", "Recents", `Settings${"Ctrl+,"}`, `Keyboard shortcuts${"Ctrl+/"}`, "Private session", "Go offline", "Check for updates", "Sign out"]);
  });

  it("Escape closes it and returns focus to the avatar", async () => {
    const user = userEvent.setup();
    render(<TopBar />);
    const avatar = screen.getByRole("button", { name: /^App menu/ });
    await user.click(avatar);
    expect(screen.getByRole("menu")).toBeInTheDocument();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
    expect(avatar).toHaveFocus();
  });

  it("a click outside closes it", async () => {
    const user = userEvent.setup();
    render(
      <>
        <TopBar />
        <p>outside</p>
      </>,
    );
    await user.click(screen.getByRole("button", { name: /^App menu/ }));
    expect(screen.getByRole("menu")).toBeInTheDocument();
    await user.click(screen.getByText("outside"));
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
  });

  it("keyboard opening focuses the first item and arrows move through the items", async () => {
    const user = userEvent.setup();
    render(<TopBar />);
    screen.getByRole("button", { name: /^App menu/ }).focus();
    await user.keyboard("{Enter}");
    expect(screen.getByRole("menuitem", { name: "Create playlist" })).toHaveFocus();
    await user.keyboard("{ArrowDown}");
    expect(screen.getByRole("menuitem", { name: "Recents" })).toHaveFocus();
    await user.keyboard("{ArrowDown}");
    expect(screen.getByRole("menuitem", { name: /Settings/ })).toHaveFocus();
    await user.keyboard("{ArrowDown}");
    expect(screen.getByRole("menuitem", { name: /Keyboard shortcuts/ })).toHaveFocus();
    await user.keyboard("{ArrowDown}");
    expect(screen.getByRole("menuitemcheckbox", { name: "Private session" })).toHaveFocus();
    await user.keyboard("{ArrowUp}{ArrowUp}{ArrowUp}{ArrowUp}{ArrowUp}");
    expect(screen.getByRole("menuitem", { name: "Sign out" })).toHaveFocus();
  });

  it("Private session is a checkable item backed by the setting", async () => {
    const set = vi.spyOn(backend, "settingsSet");
    const user = userEvent.setup();
    render(<TopBar />);
    await user.click(screen.getByRole("button", { name: /^App menu/ }));
    const item = screen.getByRole("menuitemcheckbox", { name: "Private session" });
    expect(item).toHaveAttribute("aria-checked", "false");
    await user.click(item);
    expect(set).toHaveBeenCalledWith({ privateSession: true });
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /^App menu/ }));
    expect(screen.getByRole("menuitemcheckbox", { name: "Private session" })).toHaveAttribute("aria-checked", "true");
  });

  it("Settings opens the settings page; Check for updates asks the updater", async () => {
    const check = vi.spyOn(backend, "updateCheck");
    const user = userEvent.setup();
    render(<TopBar />);
    await user.click(screen.getByRole("button", { name: /^App menu/ }));
    await user.click(screen.getByRole("menuitem", { name: /Settings/ }));
    expect(route()).toEqual({ name: "settings" });
    await user.click(screen.getByRole("button", { name: /^App menu/ }));
    await user.click(screen.getByRole("menuitem", { name: "Check for updates" }));
    expect(check).toHaveBeenCalled();
  });

  it("Sign out asks first, then signs out", async () => {
    const out = vi.spyOn(backend, "authSignOut").mockResolvedValue();
    const user = userEvent.setup();
    render(<TopBar />);
    await user.click(screen.getByRole("button", { name: /^App menu/ }));
    await user.click(screen.getByRole("menuitem", { name: "Sign out" }));
    expect(out).not.toHaveBeenCalled();
    await user.click(await screen.findByRole("button", { name: "Sign out" }));
    expect(out).toHaveBeenCalled();
  });
});

describe("sidebar", () => {
  it("is the library only: Home and Search live in the top bar", () => {
    render(<Sidebar collapsed={false} />);
    expect(screen.queryByRole("button", { name: "Home" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Search" })).not.toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Your Library" })).toBeInTheDocument();
  });
});

describe("profile picture", () => {
  const AVATAR = "/ext/https://plex.tv/users/abc/avatar?c=1";
  const face = () => document.querySelector(".topbar__avatar-face") as HTMLElement;
  const img = () => face().querySelector("img");

  beforeEach(() => {
    useApp.setState({ auth: { state: "signedIn", username: "listener", avatar: AVATAR, needsServerPick: false } });
  });

  it("shows the letter until the picture loads, then the picture", () => {
    render(<TopBar />);
    expect(face()).toHaveTextContent("L");
    expect(face()).toHaveAttribute("data-avatar", "letter");
    expect(img()).toHaveAttribute("src", artUrl(AVATAR, 32));
    fireEvent.load(img()!);
    expect(face()).toHaveAttribute("data-avatar", "image");
    expect(img()).toHaveClass("is-loaded");
    // The accessible name still comes from the username.
    expect(screen.getByRole("button", { name: "App menu, signed in as listener" })).toBeInTheDocument();
  });

  it("goes through the art scheme, never straight to plex.tv", () => {
    const spy = vi.spyOn(backend, "artUrl").mockImplementation((p, w, h) => `http://art.localhost/${encodeURIComponent(p)}?w=${w}&h=${h}`);
    render(<TopBar />);
    expect(spy).toHaveBeenCalledWith(AVATAR, expect.any(Number), expect.any(Number));
    expect(img()?.getAttribute("src")).toMatch(/^http:\/\/art\.localhost\//);
  });

  it("falls back to the letter when the picture fails", () => {
    render(<TopBar />);
    fireEvent.error(img()!);
    expect(img()).toBeNull();
    expect(face()).toHaveTextContent("L");
    expect(face()).toHaveAttribute("data-avatar", "letter");
  });

  it("shows the letter when the account has no picture", () => {
    useApp.setState({ auth: { state: "signedIn", username: "listener", avatar: null, needsServerPick: false } });
    render(<TopBar />);
    expect(img()).toBeNull();
    expect(face()).toHaveTextContent("L");
  });

  it("the setting turns it off", () => {
    render(<TopBar />);
    expect(img()).not.toBeNull();
    act(() => useSettings.setState({ settings: { ...DEFAULT_SETTINGS, showPlexAvatar: false } }));
    expect(img()).toBeNull();
    expect(face()).toHaveTextContent("L");
  });

  it("the app menu shows it next to the username", async () => {
    const user = userEvent.setup();
    render(<TopBar />);
    await user.click(screen.getByRole("button", { name: /^App menu/ }));
    const who = document.querySelector(".appmenu__who") as HTMLElement;
    expect(who).toHaveTextContent("listener");
    expect(who.querySelector("img")).toHaveAttribute("src", artUrl(AVATAR, 28));
  });
});
