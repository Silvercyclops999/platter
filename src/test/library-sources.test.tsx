import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { backend } from "../lib/backend";
import { idVariants, plainId, serverTagOf } from "../lib/ids";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS } from "../lib/types";
import { SettingsPage } from "../pages/SettingsPage";
import { ServerSwitcher } from "../shell/ServerSwitcher";
import { useApp } from "../stores/app";
import { useSettings } from "../stores/settings";

const mock = backend as MockBackend;
const ZED = "mock-friend-03/7";
const ZED_TAG = "9c0d1e2f";

beforeEach(async () => {
  mock.librariesMock.oneServer = false;
  await mock.settingsSet({ libraries: [], serverView: "all" });
  useSettings.setState({ settings: { ...DEFAULT_SETTINGS } });
  useApp.setState({ auth: { state: "signedIn", username: "listener", needsServerPick: false } });
});

async function librariesRow() {
  render(<SettingsPage />);
  const row = await waitFor(() => {
    const r = document.querySelector('[data-setting="libraries"]');
    expect(r).toBeTruthy();
    return r as HTMLElement;
  });
  const list = await within(row).findByRole("list", { name: "Servers and libraries" });
  return { row, list };
}

describe("Settings, Library: servers and libraries", () => {
  it("lists owned and shared servers with their libraries, the main one always in", async () => {
    const { row, list } = await librariesRow();
    expect(within(row).getByText("1 library")).toBeInTheDocument();
    const servers = within(list).getAllByRole("listitem").filter((li) => li.classList.contains("library-source"));
    expect(servers.map((s) => s.querySelector(".library-source__name")?.textContent)).toEqual(["Basement Server", "Cabin NAS", "Zed's Plex"]);
    expect(servers[0]).toHaveTextContent("Connected");
    expect(servers[0]).toHaveTextContent("Yours, on your network");
    expect(servers[1]).toHaveTextContent("Yours, offline");
    expect(servers[2]).toHaveTextContent("Shared with you, through the Plex relay");
    const main = within(list).getByRole("switch", { name: "Include Music on Basement Server" });
    expect(main).toBeChecked();
    expect(main).toBeDisabled();
    expect(within(list).getByRole("switch", { name: "Include Archive on Basement Server" })).not.toBeChecked();
  });

  it("includes a shared library and leaves it out again, kept for this device", async () => {
    const user = userEvent.setup();
    const { row, list } = await librariesRow();
    const zed = within(list).getByRole("switch", { name: "Include Zed's music on Zed's Plex" });
    await user.click(zed);
    expect(zed).toBeChecked();
    await waitFor(async () => expect((await mock.settingsGet()).libraries).toEqual([ZED]));
    expect(within(row).getByText("2 libraries")).toBeInTheDocument();
    await user.click(zed);
    await waitFor(async () => expect((await mock.settingsGet()).libraries).toEqual([]));
    expect(zed).not.toBeChecked();
  });
});

describe("The sidebar server switcher", () => {
  it("isn't there with only one server", async () => {
    render(<ServerSwitcher />);
    await new Promise((r) => setTimeout(r, 20));
    expect(screen.queryByRole("combobox", { name: "Show music from" })).toBeNull();
  });

  it("shows All servers and each server, and switching shows one server's music", async () => {
    await mock.settingsSet({ libraries: [ZED] });
    useSettings.setState({ settings: { ...DEFAULT_SETTINGS, libraries: [ZED] } });
    const user = userEvent.setup();
    render(<ServerSwitcher />);
    const select = await screen.findByRole("combobox", { name: "Show music from" });
    expect(within(select).getAllByRole("option").map((o) => o.textContent)).toEqual(["All servers", "Basement Server", "Zed's Plex"]);
    expect(select).toHaveValue("all");
    await user.selectOptions(select, "Zed's Plex");
    await waitFor(async () => expect((await mock.settingsGet()).serverView).toBe(ZED_TAG));
    expect(select).toHaveValue(ZED_TAG);
  });
});

describe("Item ids with their server", () => {
  it("reads the server tag and keeps ids saved before tags matching", () => {
    expect(plainId("0a1b2c3d:123")).toBe("123");
    expect(serverTagOf("0a1b2c3d:123")).toBe("0a1b2c3d");
    expect(serverTagOf("local:abc")).toBeNull();
    expect(idVariants("0a1b2c3d:123")).toEqual(["0a1b2c3d:123", "123"]);
    expect(idVariants("daily:1")).toEqual(["daily:1"]);
  });
});
