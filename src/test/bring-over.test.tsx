/**
 * Bring in music from another app (spec 27) in the UI: the door in Your
 * Library's + menu and on Home, the three steps, and each step's failure
 * faked: a file Platter doesn't know, a file with nothing usable, zero
 * matches, everything already there, Lidarr unreachable or its key
 * refused, a red safety check, and an album Lidarr can't find. The core's
 * side of each is in src-tauri/src/import/bring_tests.rs.
 */
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS } from "../lib/types";
import { BringPage } from "../pages/BringPage";
import { BringWaiting } from "../pages/BringWaiting";
import { HomeExplore } from "../pages/HomeExplore";
import { resetImportListeners } from "../pages/importShared";
import { createMenu } from "../shell/Sidebar";
import { useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";

const mock = backend as MockBackend;
const SLOW = { timeout: 4000 };

async function settings(patch: Record<string, unknown> = {}) {
  await mock.settingsSet({ ...DEFAULT_SETTINGS, ...patch });
  useSettings.setState({ settings: null });
  await loadSettings();
}

beforeEach(async () => {
  mock.bring.reset();
  mock.lidarr.reset("demo");
  resetImportListeners();
  useToasts.setState({ toasts: [] });
  useRouter.getState().reset({ name: "home" });
  await settings();
});

const toastText = () => useToasts.getState().toasts.map((t) => t.message).join("\n");

async function bringFile() {
  render(<BringPage />);
  await userEvent.click(await screen.findByRole("button", { name: "Choose file" }));
  return screen.findByTestId("bring-sentence", undefined, SLOW);
}

function drop(name: string) {
  const file = new File(["x"], name);
  fireEvent.drop(screen.getByTestId("bring-page"), { dataTransfer: { files: [file], types: ["Files"] } });
}

describe("the door", () => {
  it("is in Your Library's + menu", () => {
    const item = createMenu(null).find((i) => i.label === "Bring in music from another app");
    expect(item).toBeTruthy();
    item!.onSelect?.();
    const { entries, index } = useRouter.getState();
    expect(entries[index].route).toEqual({ name: "bring" });
  });

  it("leads Home's Explore until the first import", async () => {
    const { unmount } = render(<HomeExplore />);
    await userEvent.click(await screen.findByRole("button", { name: /Bring in music from another app/ }));
    expect(useRouter.getState().entries[useRouter.getState().index].route).toEqual({ name: "bring" });
    unmount();
    await settings({ importSources: ["streaming"] });
    render(<HomeExplore />);
    expect(screen.queryByRole("button", { name: /Bring in music from another app/ })).toBeNull();
  });
});

describe("step 1: where from", () => {
  it("offers each app with one line on getting the file", async () => {
    render(<BringPage />);
    const group = await screen.findByRole("group", { name: "Where from" });
    for (const name of ["Spotify", "Apple Music", "YouTube Music", "Last.fm", "Any CSV or M3U"]) {
      expect(within(group).getByRole("button", { name: new RegExp(name) })).toBeTruthy();
    }
    await userEvent.click(within(group).getByRole("button", { name: /Spotify/ }));
    expect(screen.getByRole("button", { name: "Open the request page" })).toBeTruthy();
    expect(screen.getByText(/usually ready in a few days/)).toBeTruthy();
    // The playlists can come over now, through a connected account.
    await userEvent.click(screen.getByRole("button", { name: "Bring playlists now, the rest when your file arrives" }));
    expect(await screen.findByTestId("bring-playlists-now")).toHaveTextContent("Liked Songs for Platter");
    await waitFor(() => expect(useSettings.getState().settings?.importSources).toContain("streaming-account"));
  });

  it("refuses a file it doesn't know, in plain words", async () => {
    render(<BringPage />);
    await screen.findByTestId("bring-drop");
    drop("notes.pdf");
    await waitFor(() => expect(toastText()).toMatch(/Platter doesn't know that file/), SLOW);
    expect(screen.queryByTestId("bring-sentence")).toBeNull();
  });

  it("says so when a file has nothing usable", async () => {
    mock.bring.scenario = "empty";
    render(<BringPage />);
    await screen.findByTestId("bring-drop");
    drop("export.csv");
    await waitFor(() => expect(toastText()).toMatch(/no songs, playlists or plays/), SLOW);
  });

  it("asks for one file at a time", async () => {
    render(<BringPage />);
    await screen.findByTestId("bring-drop");
    fireEvent.drop(screen.getByTestId("bring-page"), { dataTransfer: { files: [new File(["a"], "a.csv"), new File(["b"], "b.csv")], types: ["Files"] } });
    await waitFor(() => expect(toastText()).toMatch(/one file at a time/));
  });
});

describe("step 2: what you already have", () => {
  it("is one sentence and two numbers, with what went to your server", async () => {
    const sentence = await bringFile();
    expect(sentence).toHaveTextContent("812 of 900 liked songs are already in your library. 31 songs from 7 albums aren't.");
    expect(screen.getByText("liked songs here").previousSibling).toHaveTextContent("812");
    expect(screen.getByText("albums to get").previousSibling).toHaveTextContent("7");
    expect(screen.getByText(/Liked 812 songs and made 3 playlists/)).toBeTruthy();
    // Unsure matches go to a short review list, never blocking.
    expect(screen.getByText(/4 songs might be in your library/)).toBeTruthy();
    const lists = screen.getByRole("list", { name: "Playlists brought over" });
    expect(within(lists).getByRole("button", { name: /Road Trip/ })).toHaveTextContent("41 of 48 here");
    // The file's source is on in Settings for re-imports.
    await waitFor(() => expect(useSettings.getState().settings?.importSources).toContain("streaming"));
  });

  it("zero matches: nothing liked, playlists wait for their first song", async () => {
    mock.bring.scenario = "none";
    const sentence = await bringFile();
    expect(sentence).toHaveTextContent(/^None of the 900 liked songs are in your library yet/);
    expect(screen.queryByText(/^Liked/)).toBeNull();
    expect(screen.getByRole("button", { name: /Road Trip/ })).toHaveTextContent("Made once a song arrives");
  });

  it("everything already there: nothing to get", async () => {
    mock.bring.scenario = "all";
    const sentence = await bringFile();
    expect(sentence).toHaveTextContent("All 900 liked songs are already in your library.");
    expect(screen.queryByRole("heading", { name: /Get the rest/ })).toBeNull();
  });

  it("shows a playlist's missing songs greyed under its songs", async () => {
    await mock.bringRead("Downloads/my_data.zip");
    render(<BringWaiting playlistId="mock-bring-1" />);
    const box = await screen.findByTestId("bring-waiting");
    expect(within(box).getByRole("heading", { name: "Not in your library yet" })).toBeTruthy();
    expect(within(box).getAllByRole("listitem").length).toBeGreaterThan(0);
    render(<BringWaiting playlistId="500" />);
    expect(screen.getAllByTestId("bring-waiting")).toHaveLength(1);
  });
});

describe("step 3: get the rest", () => {
  async function getButton() {
    await bringFile();
    return screen.getByRole("button", { name: "Get the 7 albums" });
  }

  it("without Lidarr, opens a two-field setup with the address filled in", async () => {
    await userEvent.click(await getButton());
    const form = screen.getByRole("form");
    expect(within(form).getByLabelText("Address")).toHaveValue("http://192.168.1.20:8686");
    expect(within(form).getByLabelText("API key")).toHaveAttribute("placeholder", "Lidarr: Settings, General, API Key");
  });

  it("says when Lidarr can't be reached", async () => {
    mock.lidarr.reset("unreachable");
    await userEvent.click(await getButton());
    await userEvent.type(screen.getByLabelText("API key"), "abc123");
    await userEvent.click(screen.getByRole("button", { name: "Connect" }));
    expect(await screen.findByRole("alert", undefined, SLOW)).toHaveTextContent(/isn't reachable/);
    expect(screen.queryByTestId("bring-progress")).toBeNull();
  });

  it("says when the key is refused", async () => {
    mock.lidarr.reset("badKey");
    await userEvent.click(await getButton());
    await userEvent.type(screen.getByLabelText("API key"), "abc123");
    await userEvent.click(screen.getByRole("button", { name: "Connect" }));
    expect(await screen.findByRole("alert", undefined, SLOW)).toHaveTextContent(/didn't accept this API key/);
  });

  it("a red safety check stops it, and says what to change", async () => {
    mock.lidarr.reset("empty");
    await userEvent.click(await getButton());
    await userEvent.type(screen.getByLabelText("API key"), "abc123");
    await userEvent.click(screen.getByRole("button", { name: "Connect" }));
    const alert = await screen.findByRole("alert", undefined, SLOW);
    expect(alert).toHaveTextContent(/Lidarr needs a change before Platter asks it for anything/);
    expect(within(alert).getByRole("button", { name: "Open Lidarr settings" })).toBeTruthy();
    expect(screen.queryByTestId("bring-progress")).toBeNull();
  });

  it("set up and checked: asks for them, and an album it can't find stays on the list", async () => {
    mock.lidarr.reset("clear");
    await userEvent.click(await getButton());
    await userEvent.type(screen.getByLabelText("API key"), "abc123");
    await userEvent.click(screen.getByRole("button", { name: "Connect" }));
    expect(await screen.findByTestId("bring-progress", undefined, SLOW)).toHaveTextContent("6 albums on the way, 0 here so far.");
    expect(useSettings.getState().settings?.lidarrUrl).toBe("http://192.168.1.20:8686");
    const albums = screen.getByRole("list", { name: "Albums to get" });
    const punisher = within(albums).getByRole("listitem", { name: /Punisher by Phoebe Bridgers/ });
    expect(punisher).toHaveTextContent("Lidarr couldn't find it");
    // Try again once it's on MusicBrainz.
    mock.bring.notFound.clear();
    await userEvent.click(within(punisher).getByRole("button", { name: "Try again" }));
    await waitFor(() => expect(screen.getByTestId("bring-progress")).toHaveTextContent("7 albums on the way"));
  });

  it("with Lidarr ready it's one button, and arrivals count up", async () => {
    mock.lidarr.reset("ready");
    await settings({ lidarrUrl: "http://10.0.0.5:8686" });
    await userEvent.click(await getButton());
    expect(await screen.findByTestId("bring-progress")).toHaveTextContent("6 albums on the way, 0 here so far.");
    expect(screen.queryByRole("form")).toBeNull();
    mock.bring.arrive("low tide\u001fshoreline");
    mock.emit("library://changed", null);
    await waitFor(() => expect(screen.getByTestId("bring-progress")).toHaveTextContent("6 albums on the way, 1 here so far."));
  });
});
