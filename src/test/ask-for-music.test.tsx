/**
 * Requests: someone the server is shared with sees Request where the owner
 * sees Get, the owner answers on What's New, and each person's choice lives
 * on their row in People with access. Lidarr is never reached from the
 * asking side. Invented people and albums; the browser mock's fakes.
 */
import { ONLINE_EXTRAS } from "./onlineExtras";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { MockCatalogue } from "../lib/mock/catalogue";
import { DEFAULT_SETTINGS, type IncomingAsk } from "../lib/types";
import { AlbumPage } from "../pages/AlbumPage";
import { ruleFor } from "../pages/AskRows";
import { askLabel } from "../pages/RequestButton";
import { SearchPage } from "../pages/SearchPage";
import { resetCatalogueCache } from "../pages/SearchCatalogue";
import { PeopleWithAccess } from "../pages/SharingSettings";
import { SettingsPage } from "../pages/SettingsPage";
import { WhatsNewPage } from "../pages/WhatsNewPage";
import { advancedView, inMore, settingsView } from "../pages/settingsMeta";
import { WhatsNewButton } from "../shell/TopBar";
import { useApp } from "../stores/app";
import { loadAsksView, resetAsks, useAsks } from "../stores/asks";
import { resetRequests } from "../stores/requests";
import { useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import { useSharing } from "../stores/sharing";
import { useToasts } from "../stores/toasts";

const mock = backend as MockBackend;
const FIRST_FROST = MockCatalogue.albumId(14);
const SLOW = { timeout: 4000 };

const toasts = () => useToasts.getState().toasts.map((t) => t.message).join(" | ");

async function setUp(patch: Record<string, unknown> = {}) {
  mock.lidarr.reset("ready");
  mock.lidarr.setTypedOnce(true);
  await mock.settingsSet({ ...DEFAULT_SETTINGS, lidarrUrl: "http://10.0.0.5:8686", ...ONLINE_EXTRAS, ...patch });
  useSettings.setState({ settings: null });
  await loadSettings();
}

/** Signed in as someone the server is shared with. */
async function asGuest(canRequest = true) {
  mock.asks.reset("guest");
  mock.asks.canRequest = canRequest;
  await setUp({ lidarrUrl: "" });
  await loadAsksView();
}

beforeEach(async () => {
  useApp.setState({ auth: { state: "signedIn", username: "listener", needsServerPick: false } });
  useToasts.setState({ toasts: [] });
  useRouter.getState().reset();
  resetCatalogueCache();
  resetRequests();
  resetAsks();
  mock.asks.reset(null);
  await setUp();
});

afterEach(() => vi.restoreAllMocks());

describe("someone the server is shared with", () => {
  it("sees Request on an album the server doesn't have, and it goes to the owner, never to Lidarr", async () => {
    await asGuest();
    const get = vi.spyOn(mock, "lidarrGet");
    const asked = vi.spyOn(mock, "asksRequest");
    render(<AlbumPage id={FIRST_FROST} catalogue />);
    const request = await screen.findByRole("button", { name: "Request First Frost" }, SLOW);
    expect(screen.queryByRole("button", { name: /^Get/ })).toBeNull();
    expect(screen.queryByText(/Set up Lidarr/)).toBeNull();
    await userEvent.click(request);
    await waitFor(() => expect(asked).toHaveBeenCalledWith(expect.objectContaining({ releaseGroupId: FIRST_FROST, title: "First Frost", artist: "Cold Coast", song: false })));
    expect(get).not.toHaveBeenCalled();
    expect(await screen.findByRole("status", {}, SLOW)).toHaveTextContent("Requested");
    expect(toasts()).toMatch(/Requested First Frost\. You'll hear when it's here/);
    // Nobody's name is on the button.
    expect(document.body.textContent).not.toMatch(/Ask \w+/);
  });

  it("can take a request back, and Request shows again", async () => {
    await asGuest();
    render(<AlbumPage id={FIRST_FROST} catalogue />);
    await userEvent.click(await screen.findByRole("button", { name: "Request First Frost" }, SLOW));
    await userEvent.click(await screen.findByRole("button", { name: "Take back your request for First Frost" }, SLOW));
    expect(await screen.findByRole("button", { name: "Request First Frost" }, SLOW)).toBeInTheDocument();
  });

  it("follows the owner's answer: Getting it, then Ready, and a no can be asked again", async () => {
    await asGuest();
    render(<AlbumPage id={FIRST_FROST} catalogue />);
    await userEvent.click(await screen.findByRole("button", { name: "Request First Frost" }, SLOW));
    const id = useAsks.getState().view!.mine[0].id;
    mock.asks.ownerAnswers(id, true);
    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("Getting it"), SLOW);
    mock.asks.arrive(id);
    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("Ready"), SLOW);

    mock.asks.reset("guest");
    await loadAsksView();
    await userEvent.click(await screen.findByRole("button", { name: "Request First Frost" }, SLOW));
    mock.asks.ownerAnswers(useAsks.getState().view!.mine[0].id, false);
    expect(await screen.findByText("Not this time", {}, SLOW)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Request First Frost again" })).toBeInTheDocument();
  });

  it("sees neither Request nor Lidarr setup when the owner doesn't let them ask", async () => {
    await asGuest(false);
    render(<AlbumPage id={FIRST_FROST} catalogue />);
    await screen.findByRole("heading", { name: "First Frost" }, SLOW);
    await new Promise((r) => setTimeout(r, 200));
    expect(screen.queryByRole("button", { name: /Request|^Get/ })).toBeNull();
    expect(screen.queryByText(/Set up Lidarr/)).toBeNull();
  });

  it("is told why when a request can't be made (ten waiting)", async () => {
    await asGuest();
    vi.spyOn(mock, "asksRequest").mockRejectedValueOnce({ kind: "invalid", message: "You have 10 requests waiting. Take one back or wait for an answer." });
    render(<AlbumPage id={FIRST_FROST} catalogue />);
    await userEvent.click(await screen.findByRole("button", { name: "Request First Frost" }, SLOW));
    await waitFor(() => expect(toasts()).toMatch(/10 requests waiting/));
    expect(screen.getByRole("button", { name: "Request First Frost" })).toBeInTheDocument();
  });

  it("taps a song in search to request it", async () => {
    await asGuest();
    const tapped = vi.spyOn(mock, "lidarrTap");
    render(<SearchPage query="first frost" />);
    const list = await screen.findByRole("list", { name: /Results/ }, SLOW);
    const row = await within(list).findByLabelText(/^First Frost, Song/, {}, SLOW);
    expect(await within(row).findByText("Request", {}, SLOW)).toBeInTheDocument();
    await userEvent.click(within(row).getByRole("button", { name: "First Frost" }));
    await waitFor(() => expect(tapped).toHaveBeenCalledWith(expect.objectContaining({ title: "First Frost", artist: "Cold Coast" })));
    await waitFor(() => expect(toasts()).toMatch(/Requested First Frost/));
    expect(mock.asks.mine[0].item).toMatchObject({ title: "First Frost", song: true });
    expect(mock.lidarr.list()).toEqual([]);
  });

  it("sees their own requests on What's New", async () => {
    mock.asks.reset("guest", true);
    await setUp({ lidarrUrl: "" });
    render(<WhatsNewPage />);
    const section = await screen.findByRole("region", { name: "Your requests" }, SLOW);
    expect(within(section).getByText("Harbour Static")).toBeInTheDocument();
    expect(within(section).getByText("Getting it 62%")).toBeInTheDocument();
    await userEvent.click(within(section).getByRole("button", { name: "Take back your request for Harbour Static" }));
    await waitFor(() => expect(within(section).queryByText("Harbour Static")).toBeNull());
    expect(screen.queryByRole("region", { name: "Requested" })).toBeNull();
  });
});

describe("the owner", () => {
  it("sees what people requested on What's New and answers with one tap", async () => {
    mock.asks.reset("owner", true);
    render(<WhatsNewPage />);
    const section = await screen.findByRole("region", { name: "Requested" }, SLOW);
    expect(within(section).getByText("First Frost")).toBeInTheDocument();
    expect(within(section).getAllByText(/robin_listens requested/).length).toBe(2);
    const answered = vi.spyOn(mock, "asksAnswer");
    await userEvent.click(within(section).getByRole("button", { name: "Get First Frost for robin_listens" }));
    await waitFor(() => expect(answered).toHaveBeenCalledWith("in-14", true));
    await waitFor(() => expect(within(section).queryByRole("button", { name: "Get First Frost for robin_listens" })).toBeNull());
    await userEvent.click(within(section).getByRole("button", { name: "No thanks to robin_listens's request for Saltmarsh" }));
    expect(await within(section).findByText("Said no", {}, SLOW)).toBeInTheDocument();
    expect(mock.asks.incoming.find((a) => a.id === "in-21")?.state).toBe("declined");
  });

  it("confirms their own first request ever before a yes goes", async () => {
    mock.asks.reset("owner", true);
    mock.lidarr.setTypedOnce(false);
    render(<WhatsNewPage />);
    const section = await screen.findByRole("region", { name: "Requested" }, SLOW);
    await userEvent.click(within(section).getByRole("button", { name: "Get First Frost for robin_listens" }));
    expect(await screen.findByRole("dialog", {}, SLOW)).toBeInTheDocument();
    expect(mock.asks.incoming.find((a) => a.id === "in-14")?.state).toBe("requested");
  });

  it("gets a dot on the bell for requests waiting", async () => {
    mock.asks.reset("owner", true);
    render(<WhatsNewButton />);
    expect(await screen.findByRole("button", { name: "What's New, 3 requests waiting" }, SLOW)).toBeInTheDocument();
    const arrived: IncomingAsk = { ...mock.asks.incoming[0], id: "in-99", state: "requested", createdAt: Date.now() };
    mock.asks.incomingArrives(arrived);
    expect(await screen.findByRole("button", { name: "What's New, 4 requests waiting" }, SLOW)).toBeInTheDocument();
  });

  it("chooses per person: Asks first, Straight away or Can't ask", async () => {
    useSharing.setState({ people: null, overview: null });
    render(<PeopleWithAccess />);
    const pick = (await screen.findByRole("combobox", { name: "Getting music for robin_listens" }, SLOW)) as HTMLSelectElement;
    expect(pick.value).toBe("ask");
    // Nobody who hasn't accepted yet.
    expect(screen.queryByRole("combobox", { name: /jules/ })).toBeNull();
    await userEvent.selectOptions(pick, "straight");
    await waitFor(async () => expect((await mock.settingsGet()).requestRules).toEqual([{ userId: "71", rule: "straight" }]));
    await userEvent.selectOptions(pick, "ask");
    await waitFor(async () => expect((await mock.settingsGet()).requestRules).toEqual([]));
    expect(ruleFor(useSettings.getState().settings, "71")).toBe("ask");
  });

  it("doesn't offer the choice with requests turned off", async () => {
    await setUp({ requestsFromOthers: false });
    useSharing.setState({ people: null, overview: null });
    render(<PeopleWithAccess />);
    await screen.findByText("robin_listens", {}, SLOW);
    expect(screen.queryByRole("combobox", { name: /Getting music/ })).toBeNull();
  });
});

describe("settings", () => {
  beforeEach(() => {
    settingsView.inlinePages = false;
    advancedView.startOpen = false;
  });
  afterEach(() => {
    settingsView.inlinePages = true;
    advancedView.startOpen = true;
  });

  it("has the switch under Lidarr's More, on, and the finer two under Sharing's More", async () => {
    expect(inMore({ key: "requestsFromOthers", page: "lidarr" })).toBe(true);
    expect(inMore({ key: "requestWeeklyLimit" })).toBe(true);
    expect(inMore({ key: "requestGroupNotices" })).toBe(true);
    expect(DEFAULT_SETTINGS).toMatchObject({ requestsFromOthers: true, requestRules: [], requestWeeklyLimit: 20, requestGroupNotices: true });
    const user = userEvent.setup();
    render(<SettingsPage page="lidarr" />);
    const page = await screen.findByRole("region", { name: "Lidarr" });
    await user.click(await within(page).findByRole("button", { name: /^More in Lidarr/ }, SLOW));
    const on = await within(page).findByRole("switch", { name: "Let people you share with request music" }, SLOW);
    expect((on as HTMLInputElement).checked || on.getAttribute("aria-checked") === "true").toBe(true);
    await user.click(on);
    await waitFor(async () => expect((await mock.settingsGet()).requestsFromOthers).toBe(false));
  });

  it("labels each state the way the person sees it", () => {
    expect(askLabel("requested")).toBe("Requested");
    expect(askLabel("sending")).toBe("Requested");
    expect(askLabel("getting", 40)).toBe("Getting it 40%");
    expect(askLabel("ready")).toBe("Ready");
    expect(askLabel("declined")).toBe("Not this time");
  });
});
