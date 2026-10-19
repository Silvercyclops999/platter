/**
 * Your profile and the Stats tabs: the identity line, this month's numbers,
 * top artists and tracks (only you see them), playlists and friends (for
 * the people you share with, nothing published), the links out, the empty
 * state; the Stats page's tabs (a proper tab list, no hero repeats on
 * Overview, Recaps, Compare); and the monthly recaps setting's rename.
 */
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ContextMenuHost } from "../components/ContextMenu";
import { backend } from "../lib/backend";
import { metaLine, PROFILE_AUDIENCE, profileFriends, profileSections, thisMonth } from "../lib/profile";
import { importSettings, migrateSettingNames } from "../lib/settingsFile";
import { formatPlays, monthName } from "../lib/stats";
import { monthStory } from "../lib/statsStory";
import { DEFAULT_SETTINGS } from "../lib/types";
import { ProfilePage } from "../pages/ProfilePage";
import { StatsPage } from "../pages/StatsPage";
import { AppMenu } from "../shell/TopBar";
import { useApp } from "../stores/app";
import { useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import { useWrapped } from "../stores/wrapped";

const route = () => useRouter.getState().entries[useRouter.getState().index].route;

async function settings(patch: Partial<typeof DEFAULT_SETTINGS> = {}) {
  await backend.settingsSet({ ...DEFAULT_SETTINGS, ...patch });
  useSettings.setState({ settings: null });
  await loadSettings();
}

async function month() {
  const span = await backend.statsSpan();
  return { span, o: await backend.statsOverview(thisMonth(span.today), 10) };
}

beforeEach(async () => {
  await settings();
  useApp.setState({ auth: { state: "signedIn", username: "listener", needsServerPick: false } });
  useRouter.getState().reset({ name: "home" });
  useWrapped.setState({ year: null });
  vi.restoreAllMocks();
});

describe("profile rules", () => {
  it("your listening is yours; playlists and friends are for people you share with", () => {
    expect(profileSections("me")).toEqual(["numbers", "topArtists", "topTracks", "playlists", "friends", "links"]);
    expect(profileSections("friend")).toEqual(["playlists", "friends"]);
    expect(Object.values(PROFILE_AUDIENCE)).not.toContain("public");
  });

  it("words the meta line, this month and the friends list", () => {
    expect(metaLine(1, 1)).toBe("1 playlist · 1 play this month");
    expect(metaLine(12, 3400)).toBe("12 playlists · 3,400 plays this month");
    expect(metaLine(null, 5)).toBe("5 plays this month");
    expect(thisMonth("2026-10-17")).toEqual({ from: "2026-10-01", to: "2026-10-17" });
    const person = (name: string) => ({ accountId: 1, name, thumb: "/p", trackId: null, title: "", artist: "", artistId: null, albumId: null, album: "", cover: null, at: 0, now: false, inLibrary: true });
    const summary = (name: string | null) => ({ version: 1, name, artists: [], songs: [], unreleasedOk: false });
    expect(profileFriends([person("Robin"), person("robin ")], [summary("ROBIN"), summary("Sam"), summary(null)])).toEqual([
      { name: "Robin", thumb: "/p", canCompare: true },
      { name: "Sam", thumb: null, canCompare: true },
    ]);
  });
});

describe("Profile page", () => {
  it("shows who you are, this month's numbers, and opens Settings and the ⋯ menu", async () => {
    const user = userEvent.setup();
    const { o } = await month();
    const playlists = await backend.playlists();
    render(
      <>
        <ProfilePage />
        <ContextMenuHost />
      </>,
    );
    expect(await screen.findByRole("heading", { level: 1, name: "listener" })).toBeInTheDocument();
    await waitFor(() => expect(screen.getByTestId("profile-meta")).toHaveTextContent(metaLine(playlists.length, o.totals.plays)));
    const story = monthStory(o);
    await waitFor(() => expect(screen.getByTestId("profile-month")).toHaveTextContent(`You listened for ${story.time} this month`));
    const told = screen.getByTestId("profile-month");
    expect(told).toHaveTextContent(o.topArtists[0].name);
    expect(told).toHaveTextContent(`That's ${story.plays} over ${story.days}`);
    // A sentence, not a row of labelled boxes.
    expect(within(told).queryByRole("term")).toBeNull();
    await user.click(screen.getByRole("button", { name: "More options for your profile" }));
    expect(await screen.findByRole("menuitem", { name: /See all your stats/ })).toBeInTheDocument();
    await user.keyboard("{Escape}");
    await user.click(screen.getByRole("button", { name: "Settings" }));
    expect(route()).toEqual({ name: "settings" });
  });

  it("top artists and tracks this month are marked as only visible to you, with their links", async () => {
    const user = userEvent.setup();
    const { o, span } = await month();
    expect(o.topArtists.length).toBeGreaterThan(0);
    render(<ProfilePage />);
    const artists = await screen.findByRole("region", { name: "Top artists this month" });
    await waitFor(() => expect(within(artists).getAllByRole("listitem")).toHaveLength(Math.min(6, o.topArtists.length)));
    expect(within(artists).getByText("Only visible to you")).toBeInTheDocument();
    // Names and titles can repeat (two songs share a title on some days).
    expect(within(artists).getAllByText(o.topArtists[0].name).length).toBeGreaterThan(0);
    const tracks = screen.getByRole("region", { name: "Top songs this month" });
    expect(within(tracks).getByText("Only visible to you")).toBeInTheDocument();
    await waitFor(() => expect(within(tracks).getAllByText(o.topSongs[0].title).length).toBeGreaterThan(0));
    expect(within(tracks).getAllByText(formatPlays(o.topSongs[0].plays)).length).toBeGreaterThan(0);
    await user.click(within(artists).getByRole("button", { name: "Show all" }));
    expect(route()).toEqual({ name: "stats", tab: "overview", month: span.today.slice(0, 7) });
    await user.click(within(tracks).getByRole("button", { name: "Your Top Songs" }));
    expect(route()).toEqual({ name: "topSongs", year: Number(span.today.slice(0, 4)) });
  });

  it("playlists and friends show for you, with Compare for a friend who shared, and nothing is sent anywhere", async () => {
    const user = userEvent.setup();
    await settings({ statsCompare: true });
    const sends = [vi.spyOn(backend, "statsCompareSummary"), vi.spyOn(backend, "sharingShare"), vi.spyOn(backend, "settingsSet")];
    const playlists = await backend.playlists();
    render(<ProfilePage />);
    const lists = await screen.findByRole("region", { name: "Playlists" });
    await waitFor(() => expect(within(lists).getAllByRole("listitem")).toHaveLength(Math.min(6, playlists.length)));
    expect(within(lists).queryByText("Only visible to you")).toBeNull();
    const friends = await screen.findByRole("region", { name: "Friends" });
    await waitFor(() => expect(within(friends).getByText("Sam")).toBeInTheDocument());
    expect(within(friends).getByText("Robin")).toBeInTheDocument();
    // Only a friend who shared their stats can be compared with.
    expect(within(friends).queryByRole("button", { name: "Compare with Robin" })).toBeNull();
    for (const s of sends) expect(s).not.toHaveBeenCalled();
    await user.click(within(friends).getByRole("button", { name: "Compare with Sam" }));
    expect(route()).toEqual({ name: "stats", tab: "compare", friend: "Sam" });
  });

  it("friends hide when there are none to show", async () => {
    await settings({ showFriendActivity: false });
    render(<ProfilePage />);
    await screen.findByRole("region", { name: "Playlists" });
    await screen.findByTestId("profile-month");
    expect(screen.queryByRole("region", { name: "Friends" })).toBeNull();
  });

  it("links to all your stats and to Wrapped", async () => {
    const user = userEvent.setup();
    const { span } = await month();
    render(<ProfilePage />);
    const links = await screen.findByRole("navigation", { name: "More of your listening" });
    await user.click(within(links).getByRole("button", { name: /Wrapped$/ }));
    expect(useWrapped.getState().year).toBe(Number(span.today.slice(0, 4)));
    await user.click(within(links).getByRole("button", { name: "See all your stats" }));
    expect(route()).toEqual({ name: "stats" });
  });

  it("with nothing played, says how it fills up and still shows playlists", async () => {
    vi.spyOn(backend, "statsSpan").mockResolvedValue({ first: null, last: null, months: [], today: "2026-10-15" });
    const overview = vi.spyOn(backend, "statsOverview");
    render(<ProfilePage />);
    expect(await screen.findByText("Nothing played yet")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Import past plays" })).toBeInTheDocument();
    await screen.findByRole("region", { name: "Playlists" });
    expect(screen.queryByTestId("profile-month")).toBeNull();
    expect(screen.queryByRole("region", { name: "Top artists this month" })).toBeNull();
    await waitFor(() => expect(screen.getByTestId("profile-meta")).toHaveTextContent("0 plays this month"));
    expect(overview).not.toHaveBeenCalled();
  });

  it("with Stats off, keeps who you are, playlists and Wrapped", async () => {
    await settings({ statsPage: false });
    render(<ProfilePage />);
    await screen.findByRole("region", { name: "Playlists" });
    expect(screen.queryByTestId("profile-month")).toBeNull();
    expect(screen.queryByRole("button", { name: "See all your stats" })).toBeNull();
    expect(screen.getByRole("button", { name: /Wrapped$/ })).toBeInTheDocument();
  });

  it("opens from the top of the app menu and from Stats", async () => {
    const user = userEvent.setup();
    const { unmount } = render(<AppMenu />);
    await user.click(screen.getByRole("button", { name: /App menu/ }));
    expect(screen.getAllByRole("menuitem")[0]).toHaveAccessibleName("Profile");
    await user.click(screen.getByRole("menuitem", { name: "Profile" }));
    expect(route()).toEqual({ name: "profile" });
    unmount();
    render(<StatsPage />);
    await user.click(await screen.findByRole("button", { name: "Your profile" }));
    expect(route()).toEqual({ name: "profile" });
  });
});

describe("Stats tabs", () => {
  it("is a tab list: arrows move between Overview, Recaps and Compare, and the panel follows", async () => {
    const user = userEvent.setup();
    render(<StatsPage />);
    await screen.findByTestId("stats-hero");
    const list = screen.getByRole("tablist", { name: "Stats" });
    const tabs = within(list).getAllByRole("tab");
    expect(tabs.map((t) => t.textContent)).toEqual(["Overview", "Recaps", "Compare"]);
    expect(tabs[0]).toHaveAttribute("aria-selected", "true");
    expect(tabs[1]).toHaveAttribute("tabindex", "-1");
    const panel = screen.getByTestId("stats-panel-overview");
    expect(panel).toHaveAttribute("role", "tabpanel");
    expect(panel).toHaveAccessibleName("Overview");
    tabs[0].focus();
    await user.keyboard("{ArrowRight}");
    expect(within(list).getByRole("tab", { name: "Recaps" })).toHaveFocus();
    expect(await screen.findByTestId("stats-panel-recaps")).toHaveAccessibleName("Recaps");
    expect(screen.queryByTestId("stats-hero")).toBeNull();
    await user.keyboard("{End}");
    expect(await screen.findByTestId("stats-panel-compare")).toHaveAccessibleName("Compare");
    // Compare has its own period, so the range chips give way to a line saying so.
    expect(screen.queryByRole("group", { name: "Range" })).toBeNull();
    expect(screen.getByText(/last six months/)).toBeInTheDocument();
  });

  it("Overview doesn't repeat the top song and artist above the top lists", async () => {
    render(<StatsPage />);
    const hero = await screen.findByTestId("stats-hero");
    expect(within(hero).getAllByRole("listitem").map((li) => li.firstChild?.textContent)).toEqual(["Longest streak", "New artists found"]);
    // The only cover beside the sentence is the record played most.
    expect(hero.querySelectorAll(".art")).toHaveLength(1);
    expect(within(hero).getByTestId("stats-record")).toHaveTextContent(/^Played most/);
    expect(await screen.findByTestId("stats-feature")).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "Monthly recaps" })).toBeNull();
    expect(screen.queryByRole("heading", { name: "On your shelf" })).toBeNull();
  });

  it("Recaps has the range's months and Wrapped; tabs switched in place keep Back on the same tab", async () => {
    const user = userEvent.setup();
    useRouter.getState().navigate({ name: "stats" });
    render(<StatsPage />);
    await user.click(await screen.findByRole("tab", { name: "Recaps" }));
    expect(route()).toEqual({ name: "stats", tab: "recaps" });
    expect(await screen.findByRole("heading", { name: "Monthly recaps" })).toBeInTheDocument();
    expect(await screen.findByRole("heading", { name: "Wrapped and Top Songs" })).toBeInTheDocument();
    const span = await backend.statsSpan();
    // Last 4 weeks touches one or two months.
    await waitFor(() => expect(screen.getAllByTestId("recap-card").length).toBeGreaterThan(0));
    expect(screen.getAllByTestId("recap-card").length).toBeLessThanOrEqual(2);
    expect(span.months.length).toBeGreaterThan(2);
  });

  it("opens on a month, and on Compare with a friend", async () => {
    const { span } = await month();
    const m = span.months[1];
    const { unmount } = render(<StatsPage tab="overview" month={m} />);
    expect(await screen.findByRole("heading", { name: monthName(m) })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "A month" })).toHaveAttribute("aria-pressed", "true");
    unmount();
    await settings({ statsCompare: true });
    render(<StatsPage friend="Sam" />);
    expect(screen.getByRole("tab", { name: "Compare" })).toHaveAttribute("aria-selected", "true");
    expect(await screen.findByTestId("compare-view")).toBeInTheDocument();
  });
});

describe("the monthly recaps setting", () => {
  it("is monthlyStats now; a file from an earlier build carries its value over", () => {
    expect(DEFAULT_SETTINGS.monthlyStats).toBe(true);
    expect("monthlyStatsOnProfile" in DEFAULT_SETTINGS).toBe(false);
    expect(migrateSettingNames({ monthlyStatsOnProfile: false, statsPage: true })).toEqual({ monthlyStats: false, statsPage: true });
    expect(migrateSettingNames({ monthlyStatsOnProfile: false, monthlyStats: true })).toEqual({ monthlyStats: true });
    const { patch, report } = importSettings({ kind: "platter-settings", version: 1, settings: { monthlyStatsOnProfile: false } });
    expect(patch).toEqual({ monthlyStats: false });
    expect(report).toEqual({ applied: 1, skipped: [] });
  });
});
