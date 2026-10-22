/**
 * Smart playlists that nest and reach further: groups of rules with their
 * own all or any, title, track number, last skipped, playlist, sample rate
 * and bit depth, and begins with, ends with and between.
 */
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ContextMenuHost } from "../components/ContextMenu";
import { SmartEditorHost, completeRules, ruleWords } from "../components/SmartPlaylistEditor";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS, type SmartRules } from "../lib/types";
import { rulesLine } from "../pages/SmartPlaylistPage";
import { useApp } from "../stores/app";
import { newSmartPlaylist, resetCurate, startCurate } from "../stores/curate";
import { useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";

const mock = backend as MockBackend;
const base: SmartRules = { matchAll: true, rules: [], sort: "none", update: "live" };

beforeEach(async () => {
  mock.smart.reset();
  await mock.settingsSet({ ...DEFAULT_SETTINGS });
  useSettings.setState({ settings: null });
  await loadSettings();
  useApp.setState({ auth: { state: "signedIn", username: "listener", needsServerPick: false } });
  useToasts.setState({ toasts: [] });
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

async function openEditor() {
  render(
    <>
      <SmartEditorHost />
      <ContextMenuHost />
    </>,
  );
  act(() => newSmartPlaylist({}));
  return await screen.findByRole("dialog");
}

const count = () => screen.getByTestId("smart-count");

describe("groups", () => {
  it("a group counts as one rule with its own any, and saves to Plex", async () => {
    const user = userEvent.setup();
    const save = vi.spyOn(backend, "smartSave");
    const dialog = await openEditor();
    const [g1, g2] = [mock.fx.albums[0].genres[0], mock.fx.albums.find((a) => a.genres[0] !== mock.fx.albums[0].genres[0])!.genres[0]];
    await user.selectOptions(within(dialog).getByRole("combobox", { name: "Rule 1 field" }), "playCount");
    await user.selectOptions(within(dialog).getByRole("combobox", { name: "Rule 1 comparison" }), "gt");
    await user.type(within(dialog).getByRole("textbox", { name: "Rule 1 value" }), "0");
    await user.click(within(dialog).getByRole("button", { name: "Add a group" }));
    const group = within(dialog).getByRole("region", { name: "Group 1" });
    expect(within(group).getByRole("combobox", { name: "Group 1 match" })).toHaveValue("any");
    await user.type(within(group).getByRole("textbox", { name: "Group 1 rule 1 value" }), g1);
    await user.click(within(group).getByRole("button", { name: "Add a rule to group 1" }));
    await user.type(within(group).getByRole("textbox", { name: "Group 1 rule 2 value" }), g2);
    const want: SmartRules = {
      ...base,
      rules: [{ field: "playCount", op: "gt", value: "0" }],
      groups: [{ matchAll: false, rules: [{ field: "genre", op: "is", value: g1 }, { field: "genre", op: "is", value: g2 }] }],
    };
    const n = mock.smart.preview(want).count;
    const either = mock.smart.preview({ ...base, matchAll: false, rules: want.groups![0].rules }).count;
    expect(n).toBeLessThanOrEqual(either);
    await waitFor(() => expect(count()).toHaveTextContent(new RegExp(`^${n} song`)));
    expect(within(dialog).getByText("Saved in Plex, so your other Plex apps see it too.")).toBeInTheDocument();
    await user.type(within(dialog).getByRole("textbox", { name: "Name" }), "Either genre");
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(save).toHaveBeenCalled());
    expect(save.mock.calls[0][0].rules.groups).toEqual(want.groups);
    expect(save.mock.calls[0][0].platter).toBeUndefined();
  });

  it("taking out a group's last rule takes the group out", async () => {
    const user = userEvent.setup();
    const dialog = await openEditor();
    await user.click(within(dialog).getByRole("button", { name: "Add a group" }));
    // A new rule in a group starts like the one above it, ready to use.
    await user.selectOptions(within(dialog).getByRole("combobox", { name: "Group 1 rule 1 field" }), "lastPlayed");
    await user.click(within(dialog).getByRole("button", { name: "Add a rule to group 1" }));
    expect(within(dialog).getByRole("combobox", { name: "Group 1 rule 2 comparison" })).toHaveValue("inLast");
    expect(within(dialog).getByRole("combobox", { name: "Group 1 rule 2 time" })).toHaveValue("30d");
    await user.click(within(dialog).getByRole("button", { name: "Remove group 1 rule 2" }));
    await user.click(within(dialog).getByRole("button", { name: "Remove group 1 rule 1" }));
    expect(within(dialog).queryByRole("region", { name: "Group 1" })).toBeNull();
    await user.click(within(dialog).getByRole("button", { name: "Add a group" }));
    await user.click(within(dialog).getByRole("button", { name: "Remove group 1" }));
    expect(within(dialog).queryByRole("region", { name: "Group 1" })).toBeNull();
  });

  it("an empty group and half a range are still being typed, not errors", () => {
    const r = completeRules({
      ...base,
      rules: [{ field: "year", op: "between", value: "1990.." }],
      groups: [{ matchAll: false, rules: [{ field: "genre", op: "is", value: "" }] }],
    });
    expect(r.rules).toEqual([]);
    expect(r.groups).toBeUndefined();
  });

  it("the page names groups in brackets", () => {
    expect(
      rulesLine({
        ...base,
        rules: [{ field: "year", op: "between", value: "1990..1999" }],
        groups: [{ matchAll: false, rules: [{ field: "mood", op: "is", value: "Night" }, { field: "genre", op: "is", value: "Jazz" }] }],
      }),
    ).toBe("Year between 1990 and 1999, and (Mood is Night or Genre is Jazz)");
  });
});

describe("new fields and ways to match", () => {
  it("years between two numbers stay something Plex can hold", async () => {
    const user = userEvent.setup();
    const dialog = await openEditor();
    const y = mock.fx.albums.find((a) => a.year)!.year!;
    await user.selectOptions(within(dialog).getByRole("combobox", { name: "Rule 1 field" }), "year");
    await user.type(within(dialog).getByRole("textbox", { name: "Rule 1 value" }), String(y));
    await user.selectOptions(within(dialog).getByRole("combobox", { name: "Rule 1 comparison" }), "between");
    const to = within(dialog).getByRole("textbox", { name: "Rule 1 to" });
    expect(within(dialog).getByRole("textbox", { name: "Rule 1 from" })).toHaveValue(String(y));
    await user.clear(to);
    await user.type(to, String(y + 9));
    const n = mock.smart.preview({ ...base, rules: [{ field: "year", op: "between", value: `${y}..${y + 9}` }] }).count;
    expect(n).toBeGreaterThan(0);
    await waitFor(() => expect(count()).toHaveTextContent(new RegExp(`^${n} song`)));
    expect(within(dialog).getByText("Saved in Plex, so your other Plex apps see it too.")).toBeInTheDocument();
    expect(within(dialog).queryByRole("alert")).toBeNull();
  });

  it("title begins with, and track number, are offered", async () => {
    const user = userEvent.setup();
    const dialog = await openEditor();
    const t = mock.fx.tracks[0];
    await user.selectOptions(within(dialog).getByRole("combobox", { name: "Rule 1 field" }), "title");
    await user.selectOptions(within(dialog).getByRole("combobox", { name: "Rule 1 comparison" }), "beginsWith");
    await user.type(within(dialog).getByRole("textbox", { name: "Rule 1 value" }), t.title.slice(0, 3));
    const n = mock.smart.preview({ ...base, rules: [{ field: "title", op: "beginsWith", value: t.title.slice(0, 3) }] }).count;
    expect(n).toBeGreaterThan(0);
    await waitFor(() => expect(count()).toHaveTextContent(new RegExp(`^${n} song`)));
    expect(ruleWords({ field: "title", op: "beginsWith", value: "Love" })).toBe("Title begins with Love");
    expect(ruleWords({ field: "sampleRate", op: "gt", value: "48" })).toBe("Sample rate over 48 kHz");
  });

  it("a playlist rule is filled on this computer and says when there's no such playlist", async () => {
    const user = userEvent.setup();
    const dialog = await openEditor();
    await user.selectOptions(within(dialog).getByRole("combobox", { name: "Rule 1 field" }), "playlist");
    expect(within(dialog).getByRole("combobox", { name: "Rule 1 comparison" })).toHaveValue("is");
    const name = within(dialog).getByRole("combobox", { name: "Rule 1 value" });
    await user.type(name, "No such list");
    expect(await within(dialog).findByText('No playlist is called "No such list", so that rule can\'t match it.')).toBeInTheDocument();
    await waitFor(() => expect(count()).toHaveTextContent("No songs match yet"));
    expect(within(dialog).getByText(/Playlist isn't something Plex can filter on/)).toBeInTheDocument();
    const list = mock.fx.playlists.find((p) => (mock.fx.playlistTracks.get(p.id) ?? []).length > 0)!;
    await user.clear(name);
    await user.type(name, list.title);
    await waitFor(() => expect(within(dialog).queryByText(/No playlist is called/)).toBeNull());
    const n = new Set(mock.fx.playlistTracks.get(list.id)).size;
    await waitFor(() => expect(count()).toHaveTextContent(new RegExp(`^${n} song`)));
  });
});

describe("describe a playlist", () => {
  it("\"this or that, and something else\" makes a group for the either", async () => {
    const user = userEvent.setup();
    const dialog = await openDescribe();
    const genres = [...new Set(mock.fx.albums.flatMap((a) => a.genres))].filter((g) => !/\s/.test(g));
    const [a, b] = genres;
    await user.type(within(dialog).getByRole("textbox", { name: "Describe a playlist" }), `${a} or ${b}, lossless`);
    const chips = await within(dialog).findByRole("list", { name: "Understood" });
    await waitFor(() => expect(within(chips).getByText(`Genre is ${a} or Genre is ${b}`)).toBeInTheDocument());
    expect(within(chips).getByText("Lossless: yes")).toBeInTheDocument();
    const group = within(dialog).getByRole("region", { name: "Group 1" });
    expect(within(group).getByRole("combobox", { name: "Group 1 match" })).toHaveValue("any");
    expect(within(group).getByRole("textbox", { name: "Group 1 rule 2 value" })).toHaveValue(b);
    await user.click(within(chips).getByRole("button", { name: `Remove Genre is ${a} or Genre is ${b}` }));
    expect(within(dialog).queryByRole("region", { name: "Group 1" })).toBeNull();
  });
});
