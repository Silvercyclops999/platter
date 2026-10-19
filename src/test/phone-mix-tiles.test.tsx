/**
 * Home's Made for you tiles on the phone print each mix's name once, under
 * the cover; the computer's mix art still carries its name.
 */
import { render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { backend } from "../lib/backend";
import { DEFAULT_SETTINGS } from "../lib/types";
import { MixArt } from "../pages/MixCard";
import { usePhoneForm } from "../phone/form";
import { HomeMixes } from "../phone/pages/HomeExtras";
import { loadSettings } from "../stores/settings";

beforeEach(async () => {
  usePhoneForm.setState({ phone: true });
  await backend.settingsSet({ ...DEFAULT_SETTINGS, homeMixes: true });
  await loadSettings();
});

afterEach(() => {
  usePhoneForm.setState({ phone: false });
});

describe("Made for you on the phone", () => {
  it("each tile names its mix once, under the cover", async () => {
    render(<HomeMixes />);
    const shelf = await screen.findByRole("region", { name: "Made for you" }, { timeout: 4000 });
    const tiles = await waitFor(() => {
      const t = [...shelf.querySelectorAll<HTMLElement>(".ph-mix-tile")];
      expect(t.length).toBeGreaterThan(0);
      return t;
    });
    for (const tile of tiles) {
      const name = tile.querySelector(".ph-card-tile__title")!.textContent!;
      expect(tile.querySelector(".mix-art__strip, .mix-art__daylist")).toBeNull();
      expect(within(tile).getAllByText(name, { exact: true })).toHaveLength(1);
    }
  });

  it("the mix art keeps its name where nothing prints it under (the computer's cards)", () => {
    const mix = { id: "m1", kind: "daily", title: "Daily Mix 1", description: "", color: "#335", art: null, covers: [] } as never;
    const { container } = render(<MixArt mix={mix} />);
    expect(container.querySelector(".mix-art__name")?.textContent).toBe("Daily Mix 1");
  });
});
