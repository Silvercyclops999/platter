/**
 * Setup guides: every step of every guide, with the mock backend standing
 * in for the core, ends on a working state and a short "what's next".
 * Guides are skippable and resume where they were left.
 */
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS, type Settings } from "../lib/types";
import { GUIDES, LOOKUPS, guideById } from "../guides/guides";
import { GuideHost } from "../guides/GuideDialog";
import { GuideButton } from "../guides/GuideButton";
import { offerFirstRun } from "../guides/offer";
import { helpPage } from "../help/pages";
import { guides, useGuides, type GuideId } from "../stores/guides";
import { useApp } from "../stores/app";
import { useHelp } from "../stores/help";
import { useLooks } from "../stores/looks";
import { useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import { resetSocial, useSocial } from "../stores/social";
import { useUi } from "../stores/ui";
import { resetImportListeners } from "../pages/importShared";

const mock = backend as MockBackend;

beforeEach(async () => {
  guides.reset();
  mock.lidarr.reset();
  mock.social.reset();
  resetSocial();
  mock.importMock.reset();
  resetImportListeners();
  await mock.settingsSet({ ...DEFAULT_SETTINGS });
  useSettings.setState({ settings: null });
  await loadSettings();
  useRouter.getState().reset({ name: "home" });
  useUi.getState().setHelpOpen(false);
  useApp.setState({
    auth: {
      state: "signedIn",
      username: "listener",
      needsServerPick: false,
      server: { machineId: "mock-machine-01", name: "Basement Server", owned: true, online: true, connection: "local" },
    } as never,
  });
});

const dialog = () => screen.getByRole("dialog", { name: /^Setup guide/ });
const next = (user: ReturnType<typeof userEvent.setup>, last = false) =>
  user.click(within(dialog()).getByRole("button", { name: last ? "Finish" : "Next" }));

async function openGuide(id: GuideId) {
  render(<GuideHost />);
  act(() => guides.open(id));
  return within(await screen.findByRole("dialog", { name: `Setup guide: ${guideById(id).title}` }));
}

describe("every guide", () => {
  it.each(GUIDES.map((g) => [g.id, g] as const))("%s: walks every step to what's next, with help for each", async (id, g) => {
    const user = userEvent.setup();
    await openGuide(id);
    for (let i = 0; i < g.steps.length; i++) {
      const step = g.steps[i];
      expect(within(dialog()).getByRole("heading", { level: 2, name: step.title })).toBeInTheDocument();
      expect(within(dialog()).getByText(`Step ${i + 1} of ${g.steps.length}`)).toBeInTheDocument();
      expect(within(dialog()).getByText(step.text)).toBeInTheDocument();
      if (step.help) {
        const p = helpPage(step.help[0]);
        expect(p, `${id}/${step.id} help page`).toBeTruthy();
        if (step.help[1]) expect(p!.blocks.some((b) => b.t === "h" && b.id === step.help![1]), `${id}/${step.id}#${step.help[1]}`).toBe(true);
      }
      expect(guides.progress(id).step).toBe(i);
      await next(user, i === g.steps.length - 1);
    }
    expect(within(dialog()).getByRole("heading", { name: "What's next" })).toBeInTheDocument();
    expect(within(dialog()).getByText(g.doneText)).toBeInTheDocument();
    for (const n of g.next) expect(within(dialog()).getByRole("button", { name: n.label })).toBeInTheDocument();
    await user.click(within(dialog()).getByRole("button", { name: "Done" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(guides.progress(id).done).toBe(true);
    expect(guides.offerable(id)).toBe(false);
  });
});

describe("resuming and skipping", () => {
  it("closing keeps the step, and the guide opens there again", async () => {
    const user = userEvent.setup();
    await openGuide("lidarr");
    await next(user);
    await next(user);
    expect(within(dialog()).getByRole("heading", { level: 2, name: "The safety check" })).toBeInTheDocument();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).toBeNull();
    // Kept on this device.
    expect(JSON.parse(localStorage.getItem("platter.guides.v1")!).lidarr.step).toBe(2);
    render(<GuideButton id="lidarr" />);
    await user.click(screen.getByRole("button", { name: "Resume the guide: Connect Lidarr" }));
    expect(within(dialog()).getByRole("heading", { level: 2, name: "The safety check" })).toBeInTheDocument();
    await user.click(within(dialog()).getByRole("button", { name: "Back" }));
    expect(within(dialog()).getByRole("heading", { level: 2, name: "Lidarr's key" })).toBeInTheDocument();
  });

  it("Skip guide closes it and stops it offering itself", async () => {
    const user = userEvent.setup();
    await openGuide("firstRun");
    await user.click(within(dialog()).getByRole("button", { name: "Skip guide" }));
    expect(useGuides.getState().active).toBeNull();
    expect(guides.offerable("firstRun")).toBe(false);
  });

  it("More about this opens the step's help in the side panel", async () => {
    const user = userEvent.setup();
    await openGuide("lidarr");
    await next(user);
    await next(user);
    await user.click(within(dialog()).getByRole("button", { name: "More about this" }));
    expect(useUi.getState().helpOpen).toBe(true);
    expect(useHelp.getState()).toMatchObject({ slug: "lidarr", anchor: "the-safety-check" });
  });
});

describe("each guide's working parts", () => {
  it("Get started: Discord and online look-ups turn on from the guide", async () => {
    const user = userEvent.setup();
    const g = await openGuide("firstRun");
    expect(await g.findByText(/You're signed in as/)).toBeInTheDocument();
    await next(user);
    expect(await g.findByRole("list", { name: "Servers and libraries" })).toBeInTheDocument();
    await next(user);
    expect(g.getByRole("radiogroup", { name: "Ready-made looks" })).toBeInTheDocument();
    await next(user);
    await user.click(g.getByRole("switch", { name: "Show what I'm playing on Discord" }));
    await waitFor(async () => expect((await mock.settingsGet()).discordStatus).toBe(true));
    await next(user);
    const lookups = g.getByRole("switch", { name: "Look up artist info and new releases online" });
    expect(lookups).not.toBeChecked();
    await user.click(lookups);
    await waitFor(async () => {
      const s = await mock.settingsGet();
      for (const k of LOOKUPS) expect((s as Settings)[k], String(k)).toBe(true);
    });
    expect(lookups).toBeChecked();
  });

  it("Lidarr: the address and key save, and the safety check runs", async () => {
    const user = userEvent.setup();
    const g = await openGuide("lidarr");
    await user.type(g.getByRole("textbox", { name: "Lidarr's address" }), "http://10.0.0.5:8686{Enter}");
    await waitFor(async () => expect((await mock.settingsGet()).lidarrUrl).toBe("http://10.0.0.5:8686"));
    await next(user);
    await user.type(g.getByLabelText("Lidarr key"), "abc123{Enter}");
    await waitFor(async () => expect((await mock.lidarrStatus()).hasKey).toBe(true));
    await next(user);
    await user.click(g.getByRole("button", { name: "Check setup" }));
    expect(await g.findByText(/Lidarr .* answered on its home address/)).toBeInTheDocument();
    expect(g.getByRole("button", { name: "What the check's findings mean" })).toBeInTheDocument();
  });

  it("Library import: pick a service, see how to get the file, import it, see the report", async () => {
    const user = userEvent.setup();
    const g = await openGuide("import");
    const first = g.getAllByRole("switch")[0];
    await user.click(first);
    await waitFor(() => expect(useSettings.getState().settings?.importSources.length).toBe(1));
    await next(user);
    expect(g.getByRole("list")).toBeInTheDocument();
    await next(user);
    await user.click(g.getByRole("button", { name: "Choose file" }));
    expect(await g.findByRole("status")).toHaveTextContent(/./);
    await next(user);
    expect(await g.findByText(/import is in|imports are in/)).toBeInTheDocument();
  });

  it("Invite a friend: starts a session and shows the invite", async () => {
    const user = userEvent.setup();
    const g = await openGuide("sharing");
    await next(user);
    await user.click(g.getByRole("button", { name: "Start a listening session" }));
    await waitFor(() => expect(useSocial.getState().listen.role).toBe("host"));
    expect(g.getByText("Your listening session is on.")).toBeInTheDocument();
    await next(user);
    await user.click(g.getByRole("button", { name: "Show the invite link" }));
    await waitFor(() => expect(useSocial.getState().inviteOpen).toBe(true));
  });

  it("Stream Deck: installs the plugin and allows it", async () => {
    const user = userEvent.setup();
    const g = await openGuide("streamDeck");
    expect(await g.findByRole("button", { name: /Install/ })).toBeInTheDocument();
    await next(user);
    await user.click(g.getByRole("switch", { name: "Allow Stream Deck" }));
    await waitFor(async () => expect((await mock.settingsGet()).allowStreamDeck).toBe(true));
  });
});

describe("first run", () => {
  it("offers Get started after the first sign-in when guides are on", async () => {
    render(<GuideHost />);
    act(() => offerFirstRun());
    expect(await screen.findByRole("dialog", { name: "Setup guide: Get started" })).toBeInTheDocument();
    await waitFor(async () => expect((await mock.settingsGet()).lookStepDone).toBe(true));
  });

  it("offers Pick a look instead when guides are off", async () => {
    await mock.settingsSet({ showSetupGuides: false });
    await loadSettings();
    act(() => offerFirstRun());
    expect(useGuides.getState().active).toBeNull();
    expect(useLooks.getState().pick).toBe(true);
    act(() => useLooks.setState({ pick: false }));
  });
});
