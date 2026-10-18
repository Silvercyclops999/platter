import { ONLINE_EXTRAS } from "./onlineExtras";
import { act, fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Profiler } from "react";
import { beforeEach, describe, expect, it } from "vitest";
import { backend } from "../lib/backend";
import { DEFAULT_SETTINGS } from "../lib/types";
import { SettingsPage } from "../pages/SettingsPage";
import { loadSettings, useSettings } from "../stores/settings";

// Times the Settings page: the first render, a keystroke, a switch and the
// queries tests make on it. Skipped unless BENCH=1 is set:
//   BENCH=1 npx vitest run src/test/settings-bench.test.tsx --silent=false
const env = (globalThis as unknown as { process: { env: Record<string, string | undefined> } }).process.env;
const run = env.BENCH ? describe : describe.skip;
const ms = (x: number) => `${x.toFixed(1)}ms`;
const median = (xs: number[]) => [...xs].sort((a, b) => a - b)[Math.floor(xs.length / 2)];

beforeEach(async () => {
  await backend.settingsSet({ ...DEFAULT_SETTINGS, ...ONLINE_EXTRAS });
  await loadSettings();
});

run("Settings page timing", () => {
  it("render, keystroke, switch and queries", async () => {
    let commit = 0;
    const onRender = (_id: string, _phase: string, actual: number) => {
      commit += actual;
    };
    let t = performance.now();
    render(
      <Profiler id="settings" onRender={onRender}>
        <SettingsPage />
      </Profiler>,
    );
    const mount = performance.now() - t;
    const field = await screen.findByLabelText("Last.fm API key");
    const ready = performance.now() - t;
    const loadCommits = commit;

    const keys: number[] = [];
    const keyCommits: number[] = [];
    for (const ch of "abcdefgh") {
      commit = 0;
      t = performance.now();
      act(() => {
        fireEvent.change(field, { target: { value: (field as HTMLInputElement).value + ch } });
      });
      keys.push(performance.now() - t);
      keyCommits.push(commit);
    }

    const toggles: number[] = [];
    const toggleCommits: number[] = [];
    for (let i = 0; i < 8; i++) {
      const row = document.querySelector('[data-setting="showPlexAvatar"]') as HTMLElement;
      const sw = within(row).getByRole("switch");
      commit = 0;
      t = performance.now();
      act(() => {
        fireEvent.click(sw);
      });
      toggles.push(performance.now() - t);
      toggleCommits.push(commit);
    }
    await act(async () => {});
    expect(useSettings.getState().settings).not.toBeNull();

    t = performance.now();
    screen.getByLabelText("Last.fm API key");
    const byLabel = performance.now() - t;
    t = performance.now();
    screen.getByRole("button", { name: "Save secret" });
    const byRole = performance.now() - t;

    console.log(
      [
        `mount ${ms(mount)}, ready ${ms(ready)} (React ${ms(loadCommits)})`,
        `keystroke median ${ms(median(keys))} (React ${ms(median(keyCommits))})`,
        `switch median ${ms(median(toggles))} (React ${ms(median(toggleCommits))})`,
        `getByLabelText ${ms(byLabel)}, getByRole(button, name) ${ms(byRole)}`,
        `${document.querySelectorAll("[data-setting]").length} rows, ${document.querySelectorAll("*").length} elements`,
      ].join("\n"),
    );
  }, 60_000);

  it("the Last.fm sign-in steps", async () => {
    const user = userEvent.setup();
    const marks: string[] = [];
    let t = performance.now();
    const t0 = t;
    const lap = (name: string) => {
      marks.push(`${name} ${(performance.now() - t).toFixed(0)}`);
      t = performance.now();
    };
    render(<SettingsPage />);
    lap("render");
    const secret = await screen.findByLabelText("Last.fm shared secret");
    lap("find field");
    await user.type(secret, "s3");
    lap("type");
    await user.click(screen.getByRole("button", { name: "Save secret" }));
    lap("save");
    await user.click(await screen.findByRole("button", { name: "Sign in on Last.fm" }));
    lap("sign in");
    await screen.findByText(/Allow Platter on the Last.fm page/);
    await user.click(screen.getByRole("button", { name: "Finish sign-in" }));
    lap("finish");
    await screen.findAllByText(/Signed in as listener/);
    lap("signed in");
    console.log(`sign-in ${(performance.now() - t0).toFixed(0)}ms: ${marks.join(", ")}`);
  }, 60_000);
});
