/**
 * Switching Plex Home users: the window covers itself with a "Switching
 * to…" screen in the current look from the moment a user is picked until
 * the app restarts, a failed switch goes back to the page as it was with a
 * toast, and a new window is only shown once the page has drawn (the ready
 * signal).
 */
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import { afterNextPaint, pageBackground, resetReadyForTests, signalReady, toHex } from "../lib/firstPaint";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS } from "../lib/types";
import { SwitchingScreen } from "../shell/SwitchingScreen";
import { TopBar } from "../shell/TopBar";
import { useApp } from "../stores/app";
import { useSettings } from "../stores/settings";
import {
  beginSwitch,
  connectSwitching,
  resetSwitchingForTests,
  STUCK_AFTER_MS,
  switchingText,
  useSwitching,
} from "../stores/switching";
import { useToasts } from "../stores/toasts";

const mock = backend as MockBackend;

const errors = () => useToasts.getState().toasts.filter((t) => t.kind === "error").map((t) => t.message);

beforeEach(async () => {
  useSettings.setState({ settings: { ...DEFAULT_SETTINGS } });
  useApp.setState({ auth: { state: "signedIn", username: "listener", needsServerPick: false } });
  mock.homeUsersMock.notAHome = false;
  mock.homeUsersMock.failNext = null;
  if (mock.homeUsersMock.current().uuid !== "home-owner") await mock.homeSwitch("home-owner");
  resetSwitchingForTests();
  useToasts.setState({ toasts: [] });
  mock.homeUsersMock.switches.length = 0;
  mock.readySignals.length = 0;
  resetReadyForTests();
});

afterEach(() => {
  vi.useRealTimers();
  resetSwitchingForTests();
  document.body.style.background = "";
});

/** The app menu with the switching screen over it, as in the main window. */
async function openMenu() {
  const user = userEvent.setup();
  render(
    <>
      <TopBar />
      <SwitchingScreen />
    </>,
  );
  await user.click(screen.getByRole("button", { name: /^App menu/ }));
  // Home users fold out under "Switch account".
  await user.click(await screen.findByRole("menuitem", { name: "Switch account" }));
  return user;
}

describe("the switching screen", () => {
  it("stays away until a user is picked or the core says it's restarting", async () => {
    const un = connectSwitching(mock);
    render(<SwitchingScreen />);
    await act(async () => {});
    expect(screen.queryByTestId("switching-screen")).toBeNull();
    act(() => mock.emit("home://changed", { restarting: true }));
    expect(screen.getByRole("status")).toHaveTextContent("Switching users");
    un();
  });

  it("comes up as soon as a user is picked, with their name, and goes when the switch is over", async () => {
    const un = connectSwitching(mock);
    const seen: string[] = [];
    const stop = useSwitching.subscribe((s) => {
      if (s.shown) seen.push(switchingText(s.to));
    });
    const user = await openMenu();
    await user.click(await screen.findByRole("menuitem", { name: "Switch to Kids" }));
    await waitFor(() => expect(mock.homeUsersMock.current().uuid).toBe("home-kids"));
    expect(seen[0]).toBe("Switching to Kids");
    // The mock doesn't restart: the screen goes again once it's done.
    await waitFor(() => expect(screen.queryByTestId("switching-screen")).toBeNull());
    expect(errors()).toEqual([]);
    stop();
    un();
  });

  it("shows the user's picture and name over everything, in the current look", async () => {
    const un = connectSwitching(mock);
    render(<SwitchingScreen />);
    act(() => beginSwitch({ name: "Sam", avatar: null }));
    const el = screen.getByTestId("switching-screen");
    expect(el).toHaveClass("switching");
    expect(el).toHaveAttribute("aria-live", "assertive");
    expect(el).toHaveTextContent("Switching to Sam");
    expect(el.querySelector(".switching__face")).toHaveTextContent("S");
    act(() => mock.emit("home://changed", { restarting: true }));
    expect(screen.getByTestId("switching-screen")).toHaveTextContent("Switching to Sam");
    act(() => mock.emit("home://changed", { restarting: false }));
    expect(screen.queryByTestId("switching-screen")).toBeNull();
    expect(useSwitching.getState().to).toBeNull();
    un();
  });

  it("a wrong PIN goes back to the user as before, with a toast and the PIN box to try again", async () => {
    const un = connectSwitching(mock);
    const user = await openMenu();
    await user.click(await screen.findByRole("menuitem", { name: "Switch to Sam, needs a PIN" }));
    const pin = await screen.findByLabelText("PIN");
    await user.type(pin, "0000");
    await user.click(screen.getByRole("button", { name: "Switch" }));
    await screen.findByRole("alert");
    expect(screen.queryByTestId("switching-screen")).toBeNull();
    expect(useSwitching.getState()).toEqual({ to: null, shown: false, restarting: false });
    expect(errors()).toEqual(["Couldn't switch to Sam: That PIN isn't right."]);
    expect(mock.homeUsersMock.current().uuid).toBe("home-owner");
    un();
  });

  it("a switch that can't reach Plex goes back to the user as before, with a toast", async () => {
    const un = connectSwitching(mock);
    mock.homeUsersMock.failNext = { kind: "network", message: "Can't reach plex.tv." };
    const user = await openMenu();
    await user.click(await screen.findByRole("menuitem", { name: "Switch to Kids" }));
    await waitFor(() => expect(errors()).toEqual(["Couldn't switch to Kids: Can't reach plex.tv."]));
    expect(screen.queryByTestId("switching-screen")).toBeNull();
    expect(mock.homeUsersMock.current().uuid).toBe("home-owner");
    expect(useApp.getState().auth?.username).toBe("listener");
    un();
  });

  it("is never left up when the restart doesn't happen", async () => {
    vi.useFakeTimers();
    const un = connectSwitching(mock);
    render(<SwitchingScreen />);
    act(() => beginSwitch({ name: "Kids", avatar: null }));
    await act(async () => {});
    act(() => mock.emit("home://changed", { restarting: true }));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(STUCK_AFTER_MS - 1);
    });
    expect(screen.getByTestId("switching-screen")).toBeInTheDocument();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
    });
    expect(screen.queryByTestId("switching-screen")).toBeNull();
    expect(errors()[0]).toMatch(/^Couldn't switch to Kids/);
    un();
  });
});

describe("the ready signal", () => {
  it("turns the page's colours into #rrggbb", () => {
    expect(toHex("rgb(244, 241, 234)")).toBe("#f4f1ea");
    expect(toHex("rgba(0, 0, 0, 1)")).toBe("#000000");
    expect(toHex("#0E0D0C".toLowerCase())).toBe("#0e0d0c");
    expect(toHex("rgba(0, 0, 0, 0)")).toBeNull();
    expect(toHex("transparent")).toBeNull();
  });

  it("tells the core once, after the next frame, with the look's background", async () => {
    document.body.style.background = "rgb(244, 241, 234)";
    expect(pageBackground()).toBe("#f4f1ea");
    await signalReady(mock);
    await signalReady(mock);
    expect(mock.readySignals).toEqual(["#f4f1ea"]);
  });

  it("still goes when animation frames don't run", async () => {
    vi.useFakeTimers();
    const win = { setTimeout, clearTimeout, requestAnimationFrame: () => 0 } as unknown as Window;
    let done = false;
    void afterNextPaint(win, 150).then(() => (done = true));
    await vi.advanceTimersByTimeAsync(149);
    expect(done).toBe(false);
    await vi.advanceTimersByTimeAsync(1);
    expect(done).toBe(true);
  });

  it("waits for two frames when they run", async () => {
    vi.useFakeTimers();
    const frames: Array<() => void> = [];
    const win = {
      setTimeout,
      clearTimeout,
      requestAnimationFrame: (cb: () => void) => frames.push(cb),
    } as unknown as Window;
    let done = false;
    void afterNextPaint(win, 10_000).then(() => (done = true));
    frames.shift()!();
    await Promise.resolve();
    expect(done).toBe(false);
    frames.shift()!();
    await vi.advanceTimersByTimeAsync(0);
    expect(done).toBe(true);
  });

  it("a core that can't be told doesn't break the page", async () => {
    const failing = { ...mock, windowReady: () => Promise.reject(new Error("no")) } as unknown as MockBackend;
    await expect(signalReady(failing)).resolves.toBeUndefined();
  });
});
