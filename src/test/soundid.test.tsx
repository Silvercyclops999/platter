/**
 * Identify by sound in the UI: its Settings rows (off by default, what is
 * sent, the key kept in the keychain, which songs), the library doctor's
 * "sounds like" finding with the suggested tags (nothing written), and the
 * "Sounds like" hint on unreleased songs only.
 */
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { SoundsLikeHint } from "../components/SoundsLikeHint";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { ARCHIVE_ARTIST_ID } from "../lib/mock/fixtures";
import type { SoundIdStatus } from "../lib/types";
import { DEFAULT_SETTINGS } from "../lib/types";
import { DoctorPage } from "../pages/DoctorPage";
import { SettingsPage } from "../pages/SettingsPage";
import { soundIdNote } from "../pages/SoundIdSettings";
import { useApp } from "../stores/app";
import { reloadEditAccess, usePlexEdit } from "../stores/plexEdit";
import { useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";

const mock = backend as MockBackend;

async function settings(patch: Record<string, unknown> = {}) {
  await mock.settingsSet({ ...DEFAULT_SETTINGS, ...patch });
  useSettings.setState({ settings: null });
  await loadSettings();
}

beforeEach(async () => {
  mock.doctor.reset();
  mock.soundId.appKey = false;
  mock.soundId.userKey = null;
  mock.soundId.refused = false;
  usePlexEdit.setState({ dialog: null });
  await reloadEditAccess();
  useToasts.setState({ toasts: [] });
  useRouter.getState().reset();
  useApp.setState({ auth: { state: "signedIn", username: "listener", needsServerPick: false } });
  await settings();
});

const row = (key: string) => document.querySelector(`[data-setting="${key}"]`) as HTMLElement;

describe("identify by sound settings", () => {
  it("is off by default and says exactly what is sent", async () => {
    render(<SettingsPage />);
    const sw = await screen.findByRole("switch", { name: "Identify songs by sound" });
    expect(sw).not.toBeChecked();
    expect(DEFAULT_SETTINGS.identifyBySound).toBe(false);
    expect(DEFAULT_SETTINGS.soundIdScope).toBe("untagged");
    const r = row("identifyBySound");
    expect(r).toHaveTextContent("Only the fingerprint and the song's length are sent");
    expect(r).toHaveTextContent("never renamed or matched to a release");
  });

  it("turns on and picks which songs", async () => {
    const user = userEvent.setup();
    mock.soundId.appKey = true;
    render(<SettingsPage />);
    await user.click(await screen.findByRole("switch", { name: "Identify songs by sound" }));
    await waitFor(() => expect(useSettings.getState().settings?.identifyBySound).toBe(true));
    expect(within(row("soundIdScope")).getByText("Untagged only")).toBeInTheDocument();
    await mock.settingsSet({ soundIdScope: "all" });
    expect((await mock.settingsGet()).soundIdScope).toBe("all");
  });

  it("says when there's no key, and keeps the user's own key in the keychain", async () => {
    const user = userEvent.setup();
    await settings({ identifyBySound: true });
    render(<SettingsPage />);
    expect(await within(row("identifyBySound")).findByText(/has no AcoustID key/)).toBeInTheDocument();
    const field = within(row("acoustidKey")).getByLabelText("AcoustID application key");
    expect(field).toHaveAttribute("type", "password");
    await user.type(field, "abc123");
    await user.click(within(row("acoustidKey")).getByRole("button", { name: "Save key" }));
    expect(await within(row("acoustidKey")).findByRole("button", { name: "Remove key" })).toBeInTheDocument();
    expect(mock.soundId.userKey).toBe("abc123");
    await user.click(within(row("acoustidKey")).getByRole("button", { name: "Remove key" }));
    await waitFor(() => expect(mock.soundId.userKey).toBeNull());
  });

  it("refuses something that isn't a key", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    const field = await within(row("acoustidKey")).findByLabelText("AcoustID application key");
    await user.type(field, "not a key!");
    await user.click(within(row("acoustidKey")).getByRole("button", { name: "Save key" }));
    await waitFor(() => expect(useToasts.getState().toasts.some((t) => /Couldn't save the AcoustID key/.test(t.message))).toBe(true));
    expect(mock.soundId.userKey).toBeNull();
    // What was typed stays, to fix rather than type again.
    expect(field).toHaveValue("not a key!");
  });

  it("notes progress, a refused key and trouble", () => {
    const base: SoundIdStatus = { on: true, appKey: true, userKey: false, keyRefused: false, configured: true, running: false, paused: false, total: 40, done: 10, identified: 7 };
    expect(soundIdNote(base, true)?.text).toBe("Identified 7 of 10 songs checked (40 in all).");
    expect(soundIdNote({ ...base, keyRefused: true }, true)).toEqual({ text: "AcoustID refused the key. Add another one below.", error: true });
    expect(soundIdNote({ ...base, paused: true }, true)?.text).toMatch(/can't be reached/);
    expect(soundIdNote({ ...base, configured: false, appKey: false }, false)?.error).toBe(false);
    expect(soundIdNote(base, false)).toBeUndefined();
  });
});

describe("the library doctor's sounds-like finding", () => {
  it("shows the suggested tags next to the file's own and writes nothing", async () => {
    const user = userEvent.setup();
    await settings({ identifyBySound: true });
    render(<DoctorPage />);
    const title = await screen.findByRole("heading", { level: 3, name: /Untagged or mislabelled file, sounds like Night Drive by Ana/ });
    const item = title.closest("li")!;
    await user.click(within(item).getByRole("button", { name: "See suggested tags" }));
    const table = within(item).getByRole("table");
    expect(within(table).getByText("Suggested (94% match)")).toBeInTheDocument();
    expect(within(table).getByText("Night Drive")).toBeInTheDocument();
    expect(within(table).getByText("0f0e0d0c-0b0a-4909-8807-060504030201")).toBeInTheDocument();
    expect(within(item).getByText(/Platter doesn't change the file/)).toBeInTheDocument();
    expect(within(item).getByText(/Only the fingerprint and the file's length were sent/)).toBeInTheDocument();
  });

  it("isn't there while identify by sound is off, or its check is", async () => {
    render(<DoctorPage />);
    await screen.findAllByRole("heading", { level: 3 });
    expect(screen.queryByText(/sounds like Night Drive/)).toBeNull();
  });
});

describe("the sounds like hint", () => {
  const archived = () => mock.fx.tracks.find((t) => t.artistId === ARCHIVE_ARTIST_ID)!;
  const released = () => mock.fx.tracks.find((t) => t.artistId !== ARCHIVE_ARTIST_ID)!;

  it("shows on an unreleased song and opens the released copy", async () => {
    const user = userEvent.setup();
    await settings({ identifyBySound: true });
    render(<SoundsLikeHint trackId={archived().id} />);
    const r = released();
    const link = await screen.findByRole("button", { name: `${r.title} by ${r.artistTitle}` });
    expect(screen.getByText(/a hint from its sound/)).toBeInTheDocument();
    await user.click(link);
    const e = useRouter.getState().entries;
    expect(e[e.length - 1].route).toEqual({ name: "album", id: r.albumId });
  });

  it("is hidden when the setting is off and on released songs", async () => {
    const { unmount } = render(<SoundsLikeHint trackId={archived().id} />);
    await new Promise((r) => setTimeout(r, 50));
    expect(screen.queryByText(/Sounds like/)).toBeNull();
    unmount();
    await settings({ identifyBySound: true });
    render(<SoundsLikeHint trackId={released().id} />);
    await new Promise((r) => setTimeout(r, 50));
    expect(screen.queryByText(/Sounds like/)).toBeNull();
  });
});
