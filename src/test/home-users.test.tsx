import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { MOCK_HOME_PIN } from "../lib/mock/homeUsers";
import { DEFAULT_SETTINGS } from "../lib/types";
import { SettingsPage } from "../pages/SettingsPage";
import { TopBar } from "../shell/TopBar";
import { useApp } from "../stores/app";
import { useSettings } from "../stores/settings";

const mock = backend as MockBackend;

beforeEach(async () => {
  useSettings.setState({ settings: { ...DEFAULT_SETTINGS } });
  useApp.setState({ auth: { state: "signedIn", username: "listener", needsServerPick: false } });
  mock.homeUsersMock.notAHome = false;
  mock.homeUsersMock.switches.length = 0;
  mock.homeUsersMock.remember = true;
  // Back to the user who signed in.
  if (mock.homeUsersMock.current().uuid !== "home-owner") await mock.homeSwitch("home-owner");
  mock.homeUsersMock.switches.length = 0;
});

async function openMenu() {
  const user = userEvent.setup();
  render(<TopBar />);
  await user.click(screen.getByRole("button", { name: /^App menu/ }));
  // The Home users fold out under one "Switch account" row.
  const toggle = await screen.findByRole("menuitem", { name: "Switch account" });
  expect(toggle).toHaveAttribute("aria-expanded", "false");
  expect(screen.queryByRole("group", { name: "Switch Plex Home user" })).toBeNull();
  await user.click(toggle);
  expect(toggle).toHaveAttribute("aria-expanded", "true");
  const group = await screen.findByRole("group", { name: "Switch Plex Home user" });
  return { user, group };
}

describe("Plex Home users in the app menu", () => {
  it("lists the other Home users with their pictures, under the current one", async () => {
    const { group } = await openMenu();
    const names = within(group).getAllByRole("menuitem").map((b) => b.getAttribute("aria-label"));
    expect(names).toEqual(["Switch to Sam, needs a PIN", "Switch to Kids"]);
    expect(group.querySelector("img")).toBeTruthy();
    expect(document.querySelector(".appmenu__who")).toHaveTextContent("listener");
  });

  it("switches straight away to a user without a PIN", async () => {
    const { user } = await openMenu();
    await user.click(screen.getByRole("menuitem", { name: "Switch to Kids" }));
    await waitFor(() => expect(mock.homeUsersMock.current().uuid).toBe("home-kids"));
    expect(mock.homeUsersMock.switches).toEqual([{ uuid: "home-kids", withPin: false }]);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("asks for the PIN, says when it's wrong, and switches with the right one", async () => {
    const { user } = await openMenu();
    await user.click(screen.getByRole("menuitem", { name: "Switch to Sam, needs a PIN" }));
    const dialog = await screen.findByRole("dialog", { name: "Enter Sam's PIN" });
    const pin = within(dialog).getByLabelText("PIN");
    await waitFor(() => expect(pin).toHaveFocus());
    expect(pin).toHaveAttribute("type", "password");
    const go = within(dialog).getByRole("button", { name: "Switch" });
    expect(go).toBeDisabled();
    await user.type(pin, "12ab0");
    expect(pin).toHaveValue("120");
    await user.type(pin, "0");
    await user.click(go);
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("That PIN isn't right.");
    expect(pin).toHaveValue("");
    expect(mock.homeUsersMock.current().uuid).toBe("home-owner");
    await user.type(pin, `${MOCK_HOME_PIN}{Enter}`);
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(mock.homeUsersMock.current().uuid).toBe("home-sam");
    expect(mock.homeUsersMock.switches.map((s) => s.withPin)).toEqual([true, true]);
  });

  it("shows no Home users for an account that isn't a Plex Home", async () => {
    mock.homeUsersMock.notAHome = true;
    const user = userEvent.setup();
    render(<TopBar />);
    await user.click(screen.getByRole("button", { name: /^App menu/ }));
    await screen.findByRole("menu");
    await new Promise((r) => setTimeout(r, 10));
    expect(screen.queryByRole("group", { name: "Switch Plex Home user" })).toBeNull();
  });
});

describe("Remember the last Home user", () => {
  it("is in Settings for a Plex Home, and turning it off is kept for this device", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    const row = await waitFor(() => {
      const r = document.querySelector('[data-setting="homeRemember"]');
      expect(r).toBeTruthy();
      return r as HTMLElement;
    });
    const sw = within(row).getByRole("switch");
    expect(sw).toBeChecked();
    await user.click(sw);
    await waitFor(() => expect(mock.homeUsersMock.remember).toBe(false));
    expect(sw).not.toBeChecked();
  });

  it("isn't shown when the account isn't a Home", async () => {
    mock.homeUsersMock.notAHome = true;
    render(<SettingsPage />);
    await new Promise((r) => setTimeout(r, 10));
    expect(document.querySelector('[data-setting="homeRemember"]')).toBeNull();
  });
});
