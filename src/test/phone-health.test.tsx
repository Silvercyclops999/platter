/**
 * The phone has no banner, so every app state the computer shows one for
 * gets a line over the phone's tabs: no music library, a full phone, a relay
 * connection, a sign-in to redo, as well as the offline ones. Each state is
 * faked from the mock's health demos.
 */
import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import { demoHealth, HEALTH_DEMOS, type HealthDemo } from "../lib/mock/health";
import { HealthStrip } from "../phone/PhoneShell";
import { healthBanner, phoneHealthLine } from "../shell/bannerView";
import { useApp } from "../stores/app";
import { useUi } from "../stores/ui";

const SERVER = { name: "Basement Server", version: "1.42.2" };
const NOW = 1_760_000_000_000;
const h = (d: HealthDemo) => demoHealth(d, SERVER, NOW);

afterEach(() => {
  vi.restoreAllMocks();
  act(() => {
    useApp.setState({ health: null });
    useUi.getState().setSignInOpen(false);
  });
});

describe("the phone's health line", () => {
  it("every state the computer has a banner for, or that dims songs, says something on the phone", () => {
    for (const d of HEALTH_DEMOS) {
      const x = h(d);
      const line = phoneHealthLine(x, NOW);
      if (healthBanner(x, NOW) || x.cachedOnly) expect(line?.message, d).toMatch(/\S/);
      else expect(line, d).toBeNull();
    }
  });

  it("all is well: nothing over the tabs", () => {
    for (const d of ["local", "remote"] as HealthDemo[]) expect(phoneHealthLine(h(d), NOW), d).toBeNull();
    expect(phoneHealthLine(null, NOW)).toBeNull();
  });

  it("no music library, a full phone and a relay connection are worded for a phone", () => {
    expect(phoneHealthLine(h("noMusicLibrary"), NOW)).toMatchObject({ message: "Your server doesn't have a music library yet.", action: "libraryGuide" });
    expect(phoneHealthLine(h("diskFull"), NOW)?.message).toBe("This phone is almost full, so new songs can't be saved. Free up some space.");
    expect(phoneHealthLine(h("relay"), NOW)?.message).toMatch(/^Playing through Plex Relay, so quality is lower/);
    expect(phoneHealthLine(h("scanning"), NOW)?.message).toBe("Adding new music… 120 of 340");
  });

  it("a server that's starting or out of reach still says only songs on the phone play", () => {
    expect(phoneHealthLine(h("starting"), NOW)?.message).toBe("Your server is starting, so only songs on this phone play for now.");
    expect(phoneHealthLine(h("offline"), NOW)?.message).toMatch(/only songs on this phone play/);
  });

  it("no music library: the line opens the steps to add one", () => {
    act(() => useApp.setState({ health: h("noMusicLibrary") }));
    render(<HealthStrip />);
    expect(screen.getByTestId("phone-offline-strip")).toHaveTextContent("doesn't have a music library");
    fireEvent.click(screen.getByRole("button", { name: "How to add one" }));
    expect(screen.getByRole("dialog", { name: "Add a music library in Plex" })).toBeInTheDocument();
  });

  it("a sign-in that ran out: the line opens the sign-in", () => {
    act(() => useApp.setState({ health: h("signIn") }));
    render(<HealthStrip />);
    fireEvent.click(screen.getByRole("button", { name: "Sign in" }));
    expect(useUi.getState().signInOpen).toBe(true);
  });

  it("access removed: the line offers another account", () => {
    const out = vi.spyOn(backend, "authSignOut").mockResolvedValue(undefined);
    act(() => useApp.setState({ health: h("accessRemoved") }));
    render(<HealthStrip />);
    fireEvent.click(screen.getByRole("button", { name: "Use another account" }));
    expect(out).toHaveBeenCalled();
  });

  it("a full phone shows and goes away once space is back", () => {
    act(() => useApp.setState({ health: h("diskFull") }));
    render(<HealthStrip />);
    expect(screen.getByTestId("phone-offline-strip")).toHaveTextContent("This phone is almost full");
    expect(screen.queryByRole("button")).toBeNull();
    act(() => useApp.setState({ health: h("local") }));
    expect(screen.queryByTestId("phone-offline-strip")).toBeNull();
  });
});
