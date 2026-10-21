/**
 * Phone Home with nothing in the library says so, like the computer's
 * Home, instead of mixes and Explore made of nothing.
 */
import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import { DEFAULT_SETTINGS, type HomeData, type Health } from "../lib/types";
import { usePhoneForm } from "../phone/form";
import { PhoneHome } from "../phone/pages/Home";
import { useApp } from "../stores/app";
import { useLikes } from "../stores/likes";
import { useRouter } from "../stores/router";
import { loadSettings } from "../stores/settings";

const EMPTY: HomeData = { shortcuts: [], recentlyPlayed: [], jumpBackIn: [], playlists: [], recentlyAdded: [], topArtists: [] };

beforeEach(async () => {
  useRouter.getState().reset();
  usePhoneForm.setState({ phone: true });
  await backend.settingsSet({ ...DEFAULT_SETTINGS });
  await loadSettings();
  useLikes.setState({ ids: new Set() });
  vi.spyOn(backend, "home").mockResolvedValue(EMPTY);
  vi.spyOn(backend, "homeRecords").mockResolvedValue({ shown: true, resume: null, newArrivals: [], spunThisWeek: [], unfinished: [], notInAYear: [] });
});

afterEach(() => {
  vi.restoreAllMocks();
  usePhoneForm.setState({ phone: false });
});

describe("phone Home with an empty library", () => {
  it("says there's no music yet, with Try again and a way to the server, and no mixes or Explore", async () => {
    render(<PhoneHome />);
    expect(await screen.findByText("No music yet")).toBeInTheDocument();
    expect(screen.getByText(/empty or still syncing/)).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "Your mixes" })).toBeNull();
    expect(screen.queryByRole("heading", { name: "Explore" })).toBeNull();
    const home = vi.mocked(backend.home);
    const before = home.mock.calls.length;
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(home.mock.calls.length).toBeGreaterThan(before);
    fireEvent.click(screen.getByRole("button", { name: "Check the server" }));
    const r = useRouter.getState();
    expect(r.entries[r.index].route).toMatchObject({ name: "settings", row: "servers" });
  });

  it("offline with nothing kept on the phone, it says so", async () => {
    act(() => useApp.setState({ health: { ...(useApp.getState().health as Health), status: "offline", cachedOnly: true } }));
    render(<PhoneHome />);
    expect(await screen.findByText("Nothing on this phone yet")).toBeInTheDocument();
    act(() => useApp.setState({ health: { ...(useApp.getState().health as Health), status: "local", cachedOnly: false } }));
  });
});
