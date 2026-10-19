/**
 * Connect your account: the import source that reads playlists from the
 * streaming account. Its Settings block (Client ID, redirect URI, re-check
 * daily, sign out), the card on the import page, and each state the core
 * can report, faked through the mock.
 */
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS } from "../lib/types";
import { IMPORT_SOURCES, importSourceName } from "../lib/importStrings";
import { ImportPage } from "../pages/ImportPage";
import { LibraryImportSettings, importMatches } from "../pages/LibraryImportSettings";
import { accountLine } from "../pages/AccountConnect";
import { resetImportListeners, skippedLine } from "../pages/importShared";
import { loadSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";
import { useRouter } from "../stores/router";

const mock = backend as MockBackend;
const CLIENT = "0123456789abcdef0123456789abcdef";
const toasts = () => useToasts.getState().toasts.map((t) => t.message);

async function setUp(patch: Partial<typeof DEFAULT_SETTINGS>) {
  await mock.settingsSet(patch);
  await loadSettings();
}

const card = () => screen.findByTestId("account-card");
const stateLine = async () => (await card()).querySelector("[data-state]") as HTMLElement;

beforeEach(async () => {
  mock.importMock.reset();
  mock.account.reset();
  resetImportListeners();
  await mock.settingsSet({ ...DEFAULT_SETTINGS });
  useSettings.setState({ settings: null });
  await loadSettings();
  useToasts.setState({ toasts: [] });
  useRouter.getState().reset({ name: "home" });
});

describe("the source", () => {
  it("is listed after the account download, with what it brings and can't", () => {
    const ids = IMPORT_SOURCES.map((s) => s.id);
    expect(ids.indexOf("streaming-account")).toBe(ids.indexOf("streaming") + 1);
    const src = IMPORT_SOURCES.find((s) => s.id === "streaming-account")!;
    expect(src.help).toMatch(/Client ID/);
    expect(src.help).toMatch(/redirect URI/);
    expect(src.help).toMatch(/User Management/);
    expect(src.note).toMatch(/Playlists only/);
    expect(importSourceName("streaming-account")).toBe(src.name);
    expect(importMatches("client id")).toBe(true);
  });

  it("says when other people's playlists were left out", () => {
    const none = { privatePlays: 0, podcasts: 0, noMetadata: 0, shortPlays: 0, incompleteRows: 0, unknownFiles: 0, othersPlaylists: 0 };
    expect(skippedLine({ ...none, othersPlaylists: 2 })).toBe("Left out: 2 playlists by other people (the account data zip has them).");
  });
});

describe("Settings", () => {
  it("shows the Client ID, the redirect URI and Re-check daily once turned on", async () => {
    render(<LibraryImportSettings />);
    const row = await waitFor(() => document.querySelector<HTMLElement>('[data-setting="import-streaming-account"]')!);
    expect(within(row).queryByTestId("account-setup")).toBeNull();
    fireEvent.click(within(row).getByRole("switch"));
    const setup = await screen.findByTestId("account-setup");
    expect(within(setup).getByText("http://127.0.0.1/callback")).toBeTruthy();
    expect(within(setup).queryByRole("button", { name: "Choose file" })).toBeNull();
    const recheck = within(setup).getByRole("switch", { name: "Re-check daily" });
    expect(recheck.getAttribute("aria-checked")).toBe("true");
    fireEvent.click(recheck);
    await waitFor(() => expect(useSettings.getState().settings?.streamingRecheck).toBe(false));

    // No Client ID: no connect button yet.
    expect(within(setup).queryByRole("button", { name: "Connect your account" })).toBeNull();
    const field = await within(setup).findByRole("textbox", { name: "Client ID" });
    fireEvent.change(field, { target: { value: `  ${CLIENT} ` } });
    fireEvent.click(within(setup).getByRole("button", { name: "Save" }));
    expect(await within(setup).findByRole("button", { name: "Connect your account" })).toBeTruthy();
    // Kept by the core (the keychain), never in settings or shown again.
    expect(mock.account.clientId).toBe(CLIENT);
    expect(JSON.stringify(useSettings.getState().settings)).not.toContain(CLIENT);
    expect(within(setup).queryByRole("textbox", { name: "Client ID" })).toBeNull();
    expect(within(setup).getByText("Saved on this computer")).toBeTruthy();
  });

  it("refuses a Client ID that isn't one, and Remove takes it away", async () => {
    await setUp({ importSources: ["streaming-account"] });
    render(<LibraryImportSettings />);
    const field = (await screen.findByRole("textbox", { name: "Client ID" })) as HTMLInputElement;
    fireEvent.change(field, { target: { value: "not an id!" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(toasts().some((t) => t.includes("Client ID"))).toBe(true));
    expect(mock.account.clientId).toBe("");
    expect(field.value).toBe("not an id!");

    mock.account.clientId = CLIENT;
    mock.account.signedIn = true;
    cleanup();
    render(<LibraryImportSettings />);
    fireEvent.click(await screen.findByRole("button", { name: "Remove" }));
    expect(await screen.findByRole("textbox", { name: "Client ID" })).toBeTruthy();
    expect(mock.account.clientId).toBe("");
    expect(mock.account.signedIn).toBe(false);
  });

  it("signs out from Settings", async () => {
    await setUp({ importSources: ["streaming-account"] });
    mock.account.clientId = CLIENT;
    mock.account.signedIn = true;
    render(<LibraryImportSettings />);
    fireEvent.click(await screen.findByRole("button", { name: "Sign out" }));
    expect(await screen.findByRole("button", { name: "Connect your account" })).toBeTruthy();
    expect(mock.account.signedIn).toBe(false);
  });
});

describe("the import page card", () => {
  it("isn't there while the source is off", async () => {
    await setUp({ importSources: ["transfer"] });
    mock.account.clientId = CLIENT;
    render(<ImportPage />);
    await screen.findByText("Nothing imported yet.");
    expect(screen.queryByTestId("account-card")).toBeNull();
  });

  it("not set up: sends you to Settings", async () => {
    await setUp({ importSources: ["streaming-account"] });
    render(<ImportPage />);
    const c = await card();
    expect(within(c).getByText(/Liked songs, saved albums, your listening history/)).toBeTruthy();
    await waitFor(async () => expect((await stateLine()).dataset.state).toBe("notSetUp"));
    fireEvent.click(within(c).getByRole("button", { name: "Set up in Settings" }));
    expect(useRouter.getState().entries[useRouter.getState().entries.length - 1]?.route).toEqual({ name: "settings", page: "libraryImport" });
  });

  it("signing in, then reading, then the playlists are in the report", async () => {
    await setUp({ importSources: ["streaming-account"] });
    mock.account.clientId = CLIENT;
    mock.account.hold = true;
    render(<ImportPage />);
    const c = await card();
    fireEvent.click(await within(c).findByRole("button", { name: "Connect your account" }));
    await waitFor(async () => expect((await stateLine()).textContent).toMatch(/Finish signing in in your browser/));
    expect(within(c).getByRole("button", { name: "Stop" })).toBeTruthy();
    mock.account.hold = false;
    act(() => mock.account.finish());
    await waitFor(async () => expect((await stateLine()).textContent).toMatch(/Connected as Sam\. Playlists read/), { timeout: 3000 });
    // The read went into the import: its report and lists show up.
    const report = await screen.findByTestId("report-streaming-account", {}, { timeout: 3000 });
    expect(report.textContent).toMatch(/playlists by other people/);
    expect(report.textContent).toMatch(/Read from your account/);
    expect(screen.getAllByText("Road Trip").length).toBeGreaterThan(0);
  });

  it("a sign-in can be stopped", async () => {
    await setUp({ importSources: ["streaming-account"] });
    mock.account.clientId = CLIENT;
    mock.account.hold = true;
    render(<ImportPage />);
    const c = await card();
    fireEvent.click(await within(c).findByRole("button", { name: "Connect your account" }));
    fireEvent.click(await within(c).findByRole("button", { name: "Stop" }));
    expect(await within(c).findByRole("button", { name: "Connect your account" })).toBeTruthy();
    expect(mock.account.signedIn).toBe(false);
  });

  it("refused: says why and offers to try again", async () => {
    await setUp({ importSources: ["streaming-account"] });
    mock.account.clientId = CLIENT;
    mock.account.nextSignIn = "deny";
    render(<ImportPage />);
    const c = await card();
    fireEvent.click(await within(c).findByRole("button", { name: "Connect your account" }));
    const alert = await within(c).findByRole("alert");
    expect(alert.textContent).toMatch(/didn't allow Platter/);
    expect(within(c).getByRole("button", { name: "Try again" })).toBeTruthy();
  });

  it("an account the developer app doesn't allow is refused with the fix", async () => {
    await setUp({ importSources: ["streaming-account"] });
    mock.account.clientId = CLIENT;
    mock.account.signedIn = true;
    mock.account.nextRead = "refused";
    render(<ImportPage />);
    const c = await card();
    fireEvent.click(await within(c).findByRole("button", { name: "Read playlists now" }));
    expect((await within(c).findByRole("alert")).textContent).toMatch(/User Management/);
  });

  it("token expired: asks to sign in again", async () => {
    await setUp({ importSources: ["streaming-account"] });
    mock.account.clientId = CLIENT;
    mock.account.signedIn = true;
    mock.account.nextRead = "expired";
    render(<ImportPage />);
    const c = await card();
    fireEvent.click(await within(c).findByRole("button", { name: "Read playlists now" }));
    expect((await within(c).findByRole("alert")).textContent).toMatch(/sign-in ran out/);
    expect(within(c).getByRole("button", { name: "Sign in again" })).toBeTruthy();
  });

  it("rate limited: says when, and the read button waits", async () => {
    await setUp({ importSources: ["streaming-account"] });
    mock.account.clientId = CLIENT;
    mock.account.signedIn = true;
    mock.account.nextRead = "rateLimited";
    render(<ImportPage />);
    const c = await card();
    fireEvent.click(await within(c).findByRole("button", { name: "Read playlists now" }));
    expect((await within(c).findByRole("alert")).textContent).toMatch(/slow down\. Try again after/);
    expect((within(c).getByRole("button", { name: "Read playlists now" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("offline: says so and offers to try again", async () => {
    await setUp({ importSources: ["streaming-account"] });
    mock.account.clientId = CLIENT;
    mock.account.signedIn = true;
    mock.account.nextRead = "offline";
    render(<ImportPage />);
    const c = await card();
    fireEvent.click(await within(c).findByRole("button", { name: "Read playlists now" }));
    expect((await within(c).findByRole("alert")).textContent).toMatch(/offline/);
    fireEvent.click(within(c).getByRole("button", { name: "Try again" }));
    await waitFor(async () => expect((await stateLine()).dataset.state).toBe("connected"));
  });
});

describe("the state line", () => {
  const base = { redirectUri: "http://127.0.0.1/callback", name: null, message: null, retryAt: null, done: 0, total: 0, lastRead: null };
  it("reads as a sentence for every state", () => {
    expect(accountLine({ ...base, state: "reading", done: 2, total: 5 })).toBe("Reading your playlists: 2 of 5.");
    expect(accountLine({ ...base, state: "connected", name: "Sam" })).toBe("Connected as Sam. Your playlists haven't been read yet.");
    expect(accountLine({ ...base, state: "signedOut" })).toBe("Not connected.");
    for (const state of ["notSetUp", "signedOut", "signingIn", "connected", "reading", "refused", "expired", "rateLimited", "offline"] as const) {
      const line = accountLine({ ...base, state });
      expect(line.length > 5 && !line.includes(String.fromCharCode(0x2014)), state).toBe(true);
    }
  });
});
