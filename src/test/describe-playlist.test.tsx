/**
 * Describe a playlist: the create menu item and its setting, typed words
 * shown as rules with a live count, words not understood, removing a rule,
 * saving as a Platter smart playlist, and the server version when it can
 * hold the rules.
 */
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { ContextMenuHost } from "../components/ContextMenu";
import { SmartEditorHost, ruleWords } from "../components/SmartPlaylistEditor";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { mockDescribe } from "../lib/mock/curate";
import { DEFAULT_SETTINGS } from "../lib/types";
import { SettingsPage } from "../pages/SettingsPage";
import { rulesLine } from "../pages/SmartPlaylistPage";
import { Sidebar } from "../shell/Sidebar";
import { useApp } from "../stores/app";
import { newSmartPlaylist, resetCurate, startCurate, useCurate } from "../stores/curate";
import { useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";
import { useUi } from "../stores/ui";

const mock = backend as MockBackend;
const genre = () => mock.fx.albums[0].genres[0];

beforeEach(async () => {
  mock.smart.reset();
  await mock.settingsSet({ ...DEFAULT_SETTINGS });
  useSettings.setState({ settings: null });
  await loadSettings();
  useApp.setState({ auth: { state: "signedIn", username: "listener", needsServerPick: false } });
  useToasts.setState({ toasts: [] });
  useUi.setState({ menu: null });
  useRouter.getState().reset({ name: "home" });
  resetCurate();
  await startCurate();
});

async function openDescribe() {
  render(
    <>
      <SmartEditorHost />
      <ContextMenuHost />
    </>,
  );
  act(() => newSmartPlaylist({ describe: true }));
  return await screen.findByRole("dialog", { name: "Describe a playlist" });
}

describe("describe a playlist", () => {
  it("is in the create menu while its setting is on", async () => {
    const user = userEvent.setup();
    render(
      <>
        <Sidebar collapsed={false} />
        <ContextMenuHost />
      </>,
    );
    await user.click(await screen.findByRole("button", { name: "Create playlist or folder" }));
    let menu = await screen.findByRole("menu");
    await user.click(within(menu).getByRole("menuitem", { name: "Describe a playlist" }));
    expect(useCurate.getState().editor?.describe).toBe(true);
    act(() => useCurate.setState({ editor: null }));

    await backend.settingsSet({ showDescribePlaylist: false });
    await loadSettings();
    await user.click(screen.getByRole("button", { name: "Create playlist or folder" }));
    menu = await screen.findByRole("menu");
    expect(within(menu).queryByRole("menuitem", { name: "Describe a playlist" })).toBeNull();
    expect(within(menu).getByRole("menuitem", { name: "Smart playlist" })).toBeInTheDocument();
  });

  it("shows what it understood as rules with a live count, lists the rest, and saves a Platter smart playlist", async () => {
    const user = userEvent.setup();
    const dialog = await openDescribe();
    await user.type(within(dialog).getByRole("textbox", { name: "Describe a playlist" }), `${genre()} from the 2010s, lossless, with kazoo solos`);
    const chips = await within(dialog).findByRole("list", { name: "Understood" });
    await waitFor(() => expect(within(chips).getAllByRole("listitem")).toHaveLength(3));
    expect(within(chips).getByText(`Genre is ${genre()}`)).toBeInTheDocument();
    expect(within(chips).getByText("Decade is 2010s")).toBeInTheDocument();
    expect(within(chips).getByText("Lossless: yes")).toBeInTheDocument();
    expect(within(dialog).getByTestId("describe-unknown")).toHaveTextContent("Didn't understand: kazoo solos");
    expect(within(dialog).getByText(/Nothing is sent anywhere/)).toBeInTheDocument();
    // The rules are editable below, and the count follows them.
    expect(within(dialog).getByRole("textbox", { name: "Rule 1 value" })).toHaveValue(genre());
    const want = mock.smart.preview({ matchAll: true, rules: [{ field: "genre", op: "is", value: genre() }, { field: "decade", op: "is", value: "2010" }], sort: "none", update: "live" }).count;
    await user.click(within(chips).getByRole("button", { name: "Remove Lossless: yes" }));
    await waitFor(() => expect(within(chips).getAllByRole("listitem")).toHaveLength(2));
    await waitFor(() => expect(screen.getByTestId("smart-count")).toHaveTextContent(`${want} song`));
    // The description names it.
    expect(within(dialog).getByRole("textbox", { name: "Name" })).toHaveValue(`${genre()} from the 2010s, lossless, with kazoo solos`);
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(useCurate.getState().smart).toHaveLength(1));
    expect(useCurate.getState().smart[0].id.startsWith("smart:")).toBe(true);
  });

  it("offers the server version only when the server can hold the rules", async () => {
    const user = userEvent.setup();
    const dialog = await openDescribe();
    const box = within(dialog).getByRole("textbox", { name: "Describe a playlist" });
    await user.type(box, `${genre()} from 2016`);
    const server = await within(dialog).findByRole("checkbox", { name: /Save it on the server instead/ });
    expect(within(dialog).getByText("A Platter smart playlist, filled on this computer.")).toBeInTheDocument();
    // Lossless is filled on this computer only: no server offer.
    await user.type(box, ", lossless");
    await waitFor(() => expect(within(dialog).queryByRole("checkbox", { name: /Save it on the server instead/ })).toBeNull());
    await user.clear(box);
    await user.type(box, `${genre()} from 2016`);
    await user.click(await within(dialog).findByRole("checkbox", { name: /Save it on the server instead/ }));
    expect(server).toBeDefined();
    await user.clear(within(dialog).getByRole("textbox", { name: "Name" }));
    await user.type(within(dialog).getByRole("textbox", { name: "Name" }), "On the server");
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(mock.fx.playlists.some((p) => p.title === "On the server" && p.smart)).toBe(true));
    expect(useCurate.getState().smart).toHaveLength(0);
  });

  it("opens from the smart playlist editor too", async () => {
    const user = userEvent.setup();
    render(<SmartEditorHost />);
    act(() => newSmartPlaylist());
    const dialog = await screen.findByRole("dialog", { name: "New smart playlist" });
    await user.click(within(dialog).getByRole("button", { name: "Describe a playlist" }));
    expect(within(dialog).getByRole("textbox", { name: "Describe a playlist" })).toBeInTheDocument();
  });

  it("names rules in words, ranges as local days", () => {
    expect(ruleWords({ field: "mood", op: "is", value: "Chill" })).toBe("Mood is Chill");
    expect(ruleWords({ field: "duration", op: "lt", value: "3" })).toBe("Length under 3 min");
    expect(ruleWords({ field: "lastPlayed", op: "notInLast", value: "3m" })).toBe("Last played not in the last 3 months");
    const range = ruleWords({ field: "played", op: "between", value: "2026-06-01..2026-08-31" });
    expect(range).toMatch(/^Played between .*2026 and .*2026$/);
    expect(range).toContain(new Date(2026, 5, 1).toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" }));
    expect(rulesLine({ matchAll: true, rules: [{ field: "played", op: "between", value: "2026-06-01..2026-08-31" }], sort: "none", update: "live" })).toContain("Played between");
  });

  it("the mock reads the brief's phrases", () => {
    const d = mockDescribe("albums from the 90s I never finished", { genre: [], artist: [], album: [] });
    expect(d.rules.rules.map((r) => `${r.field} ${r.op} ${r.value}`)).toEqual(["releaseType is albums", "decade is 1990", "finished is false"]);
    expect(mockDescribe("blorp", { genre: [], artist: [], album: [] }).unknown).toEqual(["blorp"]);
  });
});

describe("settings", () => {
  it('Show "Describe a playlist" is on by default and switches off', async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    const sw = await screen.findByRole("switch", { name: 'Show "Describe a playlist"' });
    expect(sw).toBeChecked();
    await user.click(sw);
    await waitFor(async () => expect((await backend.settingsGet()).showDescribePlaylist).toBe(false));
  });
});
